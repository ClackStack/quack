//! Keeping requests inside the budget: pruning, then compaction
//! (docs/plan/05-context-management.md). Both are recorded in the session log, so a resume
//! rebuilds the same history.

use quack_config::settings::CacheStrategy;
use quack_context::compact;
use quack_context::prune::{self, Access};
use quack_protocol::{ContextItem, ContextReport, ContextSegment, ContextZone, Event, PruneReport};
use quack_providers::{FinishReason, Message, ResponseBuilder, Sealed, ToolCall, tokens};
use quack_tools::Subject;
use serde_json::{Value, json};

use crate::session;
use crate::startup::Setup;
use crate::turn::{self, TurnCtx, raw_estimate};

/// Share of the budget that history kept verbatim may use after a compaction.
const KEEP_RATIO: f64 = 0.35;
/// Messages at the end of history that pruning leaves alone.
const PROTECT_MESSAGES: usize = 8;
/// Assumed budget when the model's window is unknown.
const FALLBACK_BUDGET: u32 = 32_000;

/// Prefill speed (tokens/s) below which `cache_strategy = "auto"` prunes superseded
/// tool results eagerly rather than re-prefilling stale reads on every step.
const SLOW_PREFILL_TPS: u32 = 400;

/// Whether `fit` should prune superseded reads/writes eagerly before reaching `threshold`.
fn eager_prune(s: &Setup) -> bool {
    match s.context.cache_strategy {
        CacheStrategy::Aggressive => true,
        CacheStrategy::Preserve => false,
        CacheStrategy::Auto => {
            !s.model.serving.prefix_cache
                || s.model
                    .serving
                    .prefill_tps
                    .is_some_and(|tps| tps > 0 && tps < SLOW_PREFILL_TPS)
        }
    }
}

/// Fit each request to the window. Native compaction keeps the prefix intact until it
/// reaches the server, unless eager pruning is active (`aggressive`, or `auto` on a
/// server without prefix caching or with slow prefill). Returns the post-fit raw estimate
/// so the caller does not have to recompute it.
pub(crate) async fn fit(ctx: &TurnCtx, history: &mut Vec<Message>) -> u32 {
    let s = &ctx.setup;
    let Some(threshold) = threshold(s) else {
        return raw_estimate(s, history);
    };
    let eager = eager_prune(s);
    if !eager {
        let raw = raw_estimate(s, history);
        if turn::scale(s, raw) <= threshold {
            return raw;
        }
    }
    if eager || !s.provider.native_compaction() {
        prune_now(ctx, history);
    }
    let mut raw = raw_estimate(s, history);
    if turn::scale(s, raw) > threshold {
        let reason = format!("context is over {}% of the budget", percent(s));
        match compact_now(ctx, history, compact::KEEP_TURNS, None, &reason).await {
            Ok(()) => raw = raw_estimate(s, history),
            Err(e) => {
                ctx.emit(Event::Warning {
                    message: format!("could not compact: {e}"),
                });
            }
        }
    }
    raw
}

fn percent(s: &Setup) -> u32 {
    (s.context.compaction_threshold * 100.0).round() as u32
}

/// Apply the pruning rules to everything but the most recent messages.
pub(crate) fn prune_now(ctx: &TurnCtx, history: &mut [Message]) {
    let protect_from = history.len().saturating_sub(PROTECT_MESSAGES);
    let ops = plan_rule_ops(&ctx.setup, history, protect_from);
    apply_prune_ops(
        &ctx.setup,
        history,
        ops,
        ctx.log.as_ref(),
        &ctx.tx,
        Some(&ctx.stats),
    );
}

/// Apply manual or rule-based pruning triggered between turns (`Op::PruneContext`).
pub(crate) fn prune_selected(
    setup: &Setup,
    history: &mut [Message],
    indices: Option<&[usize]>,
    log: Option<&session::SharedLog>,
    tx: &tokio::sync::broadcast::Sender<Event>,
) -> PruneReport {
    let ops = match indices {
        Some(idxs) => prune::manual_indices(history, idxs),
        None => {
            let protect_from = history.len().saturating_sub(PROTECT_MESSAGES);
            plan_rule_ops(setup, history, protect_from)
        }
    };
    apply_prune_ops(setup, history, ops, log, tx, None)
}

fn plan_rule_ops(setup: &Setup, history: &[Message], protect_from: usize) -> Vec<prune::PruneOp> {
    let root = &setup.tool_ctx.root;
    let hooks = prune::Hooks {
        classify: &|c| classify(setup, c),
        stale: &|p| setup.tool_ctx.reads.is_stale(std::path::Path::new(p)),
        outline: &|p, text| {
            let path = std::path::Path::new(p);
            setup
                .tool_ctx
                .grammars
                .outline(path, text)
                .filter(|o| !o.symbols.is_empty())
                .map(|o| o.render(&quack_tools::paths::display(root, path)))
        },
    };
    prune::plan(history, protect_from, &hooks)
}

fn apply_prune_ops(
    setup: &Setup,
    history: &mut [Message],
    ops: Vec<prune::PruneOp>,
    log: Option<&session::SharedLog>,
    tx: &tokio::sync::broadcast::Sender<Event>,
    stats: Option<&std::sync::Mutex<crate::stats::Stats>>,
) -> PruneReport {
    if ops.is_empty() {
        return PruneReport::default();
    }
    let removed = prune::apply(history, &ops);
    quack_providers::unseal(history);
    session::record(log, "prune", json!({ "ops": ops }));
    let items = ops
        .iter()
        .map(|o| o.content.lines().next().unwrap_or_default().to_owned())
        .collect();
    let report = PruneReport {
        removed_tokens: turn::bytes_to_tokens(removed),
        items,
    };
    let ev = Event::ContextPruned {
        report: report.clone(),
    };
    if let Some(stats) = stats
        && let Ok(mut st) = stats.lock()
    {
        st.observe(&ev);
    }
    let _ = tx.send(ev);
    let _ = setup;
    report
}

/// What a call did to files, through the same mapping and tools that ran it.
fn classify(s: &Setup, call: &ToolCall) -> Access {
    let Some((internal, args)) = turn::internal_call(s, call) else {
        return Access::Other;
    };
    let Some(tool) = s.registry.get(&internal) else {
        return Access::Other;
    };
    match tool.subject(&args, &s.tool_ctx) {
        // `glob`, `grep` and `list_dir` are reads too, but of many files.
        // A symbol read is a part, and a large file read without a range was its outline:
        // neither makes an earlier read of that file out of date.
        Ok(Subject::Read(p))
            if internal == "read"
                && args.get("symbol").is_none()
                && (read_lines(&args).is_some() || !outlined(s, &p)) =>
        {
            Access::Read {
                path: p.display().to_string(),
                lines: read_lines(&args),
            }
        }
        Ok(sub @ (Subject::Edit(_) | Subject::Edits(_))) => Access::Write {
            paths: sub
                .edited()
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
        },
        _ => Access::Other,
    }
}

/// Whether a whole-file `read` of `p` returns its outline instead.
fn outlined(s: &Setup, p: &std::path::Path) -> bool {
    let above = s.tool_ctx.outline_above;
    above > 0
        && quack_tools::outline::supported(p)
        && std::fs::metadata(p).is_ok_and(|m| m.len() as usize > above)
}

fn read_lines(args: &Value) -> Option<(u64, u64)> {
    let n = |k: &str| args.get(k).and_then(Value::as_u64);
    match (n("offset"), n("limit")) {
        (None | Some(0 | 1), None) => None,
        (offset, limit) => {
            let first = offset.unwrap_or(1).max(1);
            Some((first, limit.map_or(u64::MAX, |l| first + l.max(1) - 1)))
        }
    }
}

/// Effective token budget for compaction sizing: capped by `model.compact_at` when a
/// model compacts earlier than its full window (for example below a pricing cliff).
fn compaction_budget(s: &Setup) -> u32 {
    match (s.budget, s.model.compact_at) {
        (Some(budget), Some(cap)) => budget.min(cap),
        (Some(budget), None) => budget,
        (None, Some(cap)) => cap,
        (None, None) => FALLBACK_BUDGET,
    }
}

/// Summarize history before the cut point with the session's model and splice the summary
/// in. `keep_turns` recent turns stay verbatim when they fit.
pub(crate) async fn compact_now(
    ctx: &TurnCtx,
    history: &mut Vec<Message>,
    keep_turns: usize,
    focus: Option<&str>,
    reason: &str,
) -> Result<(), String> {
    let s = &ctx.setup;
    let budget = compaction_budget(s);
    let keep = (f64::from(budget) * KEEP_RATIO) as u32;
    let native = s.provider.native_compaction();
    let scale = |h: &[Message]| turn::scale(s, turn::history_tokens(h, native));
    let cut = compact::cut_point(history, keep_turns, keep, scale)
        .ok_or("nothing to compact yet: the conversation is only the current turn")?;
    let hooked = ctx
        .hooks(
            quack_config::hooks::HookEvent::PreCompact,
            serde_json::json!({ "reason": reason, "focus": focus }),
            crate::hooks::Target::None,
        )
        .await;
    ctx.emit(Event::Compacting {
        reason: reason.to_owned(),
    });

    let focus = focus.map(str::trim).filter(|f| !f.is_empty());
    let extra = hooked.context_text();
    if s.provider.native_compaction() && focus.is_none() && extra.is_none() {
        match compact_native(ctx, &history[..cut]).await {
            Ok(sealed) => {
                let summary = compact::fallback_summary(&history[..cut]);
                finish_compaction(ctx, history, cut, &summary, reason, Some(sealed));
                return Ok(());
            }
            Err(e) if ctx.cancel.is_cancelled() => return Err(e),
            Err(e) => ctx.emit(Event::Warning {
                message: format!("native compaction failed ({e}) · using a text summary"),
            }),
        }
    }

    let mut prompt = s
        .profile
        .compact_prompt
        .clone()
        .unwrap_or_else(|| compact::DEFAULT_PROMPT.to_owned());
    if let Some(f) = focus {
        prompt.push_str(&format!("\n\nPay particular attention to: {f}"));
    }
    if let Some(extra) = extra {
        prompt.push_str("\n\n");
        prompt.push_str(&extra);
    }
    let sum_setup = ctx
        .pool
        .small_setup(ctx)
        .await
        .unwrap_or_else(|| ctx.setup.clone());
    let sum_budget = sum_setup.budget.unwrap_or(budget).min(budget);
    // About 60% of the budget in characters, leaving room for the prompt and the reply.
    let text = compact::transcript(&history[..cut], sum_budget as usize * 2);
    let messages = vec![
        Message::System { content: prompt },
        Message::User {
            content: text,
            images: Vec::new(),
        },
    ];
    let mut req = turn::utility_request(&sum_setup, messages);
    req.cache_key = Some(turn::cache_key(ctx));
    // A looping model (small ones sometimes think forever) fails fast instead.
    let cap = (sum_budget / 4).clamp(1_024, 8_192);
    req.max_tokens = Some(sum_setup.model.max_output.map_or(cap, |m| m.min(cap)));
    let mut result = summarize(ctx, &sum_setup, req.clone()).await;
    if let Ok(text) = &result
        && let Some(why) = compact::unusable_summary(text)
    {
        // Once more, saying plainly that there are no tools; then the plain list.
        if let Some(Message::System { content }) = req.messages.first_mut() {
            content.push_str(
                "\n\nYou cannot call tools here. Reply with the summary as plain markdown text.",
            );
        }
        result =
            summarize(ctx, &sum_setup, req)
                .await
                .and_then(|text| match compact::unusable_summary(&text) {
                    Some(again) => Err(format!("{why}, twice ({again})")),
                    None => Ok(text),
                });
    }
    let summary = match result {
        Ok(text) => text,
        Err(e) if ctx.cancel.is_cancelled() => return Err(e),
        Err(e) => {
            ctx.emit(Event::Warning {
                message: format!(
                    "the model could not write a summary ({e}); keeping a plain list of \
                     earlier requests and tool calls instead"
                ),
            });
            compact::fallback_summary(&history[..cut])
        }
    };

    finish_compaction(ctx, history, cut, &summary, reason, None);
    Ok(())
}

async fn compact_native(ctx: &TurnCtx, prefix: &[Message]) -> Result<Sealed, String> {
    let s = &ctx.setup;
    let mut req = turn::request(s, turn::history_messages(s, prefix), true);
    req.cache_key = Some(turn::cache_key(ctx));
    // Threshold-based plan compaction generates a reply after compacting that Quack discards;
    // drop to the model's lowest supported effort so we don't waste high thinking on it.
    if s.provider.is_chatgpt_plan()
        && let Some(effort) = turn::lowest_effort(&s.model)
    {
        req.reasoning_effort = Some(effort);
    }
    ctx.start_request();
    let sent = std::time::Instant::now();
    let result = tokio::select! {
        biased;
        () = ctx.cancel.cancelled() => Err("interrupted".into()),
        result = s.provider.compact(req, ctx.cancel.child_token()) => result.map_err(|e| format!("{}: {e}", s.provider.id)),
    };
    ctx.record_request(
        result.as_ref().ok().and_then(|(_, usage)| usage.as_ref()),
        sent.elapsed(),
    );
    result.map(|(sealed, _)| sealed)
}

fn finish_compaction(
    ctx: &TurnCtx,
    history: &mut Vec<Message>,
    cut: usize,
    summary: &str,
    reason: &str,
    native: Option<Sealed>,
) {
    let request = compact::carried_request(history, cut);
    let message = compact::summary_message(summary, request, &turn::changed_files(ctx));
    let summary_tokens = native.as_ref().map_or_else(
        || tokens::estimate(summary),
        |sealed| turn::sealed_compaction_tokens(&sealed.items),
    );
    session::record(
        ctx.log.as_ref(),
        "compaction",
        json!({ "cut": cut, "message": message, "reason": reason, "native": native }),
    );
    session::apply_compaction(history, cut, message, native);
    // Earlier reminder messages may have been compacted away; let the next step refresh them.
    ctx.reset_reminders();
    ctx.emit(Event::Compacted {
        summary_tokens,
        replaced: u32::try_from(cut).unwrap_or(u32::MAX),
    });
    ctx.context_estimate(history);
}

/// Stream a request with no tools and return its text.
async fn summarize(
    ctx: &TurnCtx,
    s: &Setup,
    req: quack_providers::ChatRequest,
) -> Result<String, String> {
    ctx.start_request();
    let mut rx = s
        .provider
        .stream(req, turn::reasoning_format(s), ctx.cancel.child_token());
    let mut response = ResponseBuilder::default();
    let sent = std::time::Instant::now();
    let result = async {
        loop {
            let item = tokio::select! {
                () = ctx.cancel.cancelled() => return Err("interrupted".into()),
                item = rx.recv() => item,
            };
            match item {
                None => break,
                Some(Ok(event)) => response.push(&event),
                Some(Err(e)) => return Err(format!("{}: {e}", s.provider.id)),
            }
        }
        Ok(())
    }
    .await;
    ctx.record_request_for(s, response.usage.as_ref(), sent.elapsed());
    result?;
    match &response.finish {
        Some(FinishReason::Stop) => {}
        Some(reason) => return Err(format!("the summary did not finish: {reason:?}")),
        None => return Err("the summary stream ended without a finish reason".into()),
    }
    let Message::Assistant { content: text, .. } = response.into_message() else {
        return Err("the provider returned no summary".into());
    };
    let text = text.trim();
    if text.is_empty() {
        return Err("the model returned an empty summary".into());
    }
    Ok(text.to_owned())
}

/// Requests estimated above this get pruned, then compacted. Honors a model's `compact_at`
/// soft ceiling (e.g. below a long-context pricing cliff) when lower than the budget ratio.
pub(crate) fn threshold(s: &Setup) -> Option<u32> {
    let by_ratio = s
        .budget
        .map(|b| (f64::from(b) * s.context.compaction_threshold.clamp(0.1, 1.0)) as u32);
    match (by_ratio, s.model.compact_at) {
        (Some(limit), Some(cap)) => Some(limit.min(cap)),
        (Some(limit), None) => Some(limit),
        (None, Some(cap)) => Some(cap),
        (None, None) => None,
    }
}

/// What is using the context. `history` is `None` while a turn holds it.
pub(crate) fn report(s: &Setup, history: Option<&[Message]>) -> ContextReport {
    let ratio = s.estimator.lock().map_or(1.0, |e| e.ratio());
    let tok = |text: &str| (f64::from(tokens::estimate(text)) * ratio).round() as u32;
    let mut segments = vec![ContextSegment {
        name: "System prompt".into(),
        tokens: tok(&s.system),
        detail: format!("profile {}, core prompt", s.profile.id),
        zone: Some(ContextZone::CachedPrefix),
        prunable_indices: Vec::new(),
    }];
    if !s.session_context.is_empty() {
        segments.push(ContextSegment {
            name: "Session context".into(),
            tokens: tok(&s.session_context),
            detail: "workspace rules, memory & env".into(),
            zone: Some(ContextZone::SessionContext),
            prunable_indices: Vec::new(),
        });
    }
    segments.push(ContextSegment {
        name: "Tool definitions".into(),
        tokens: tok(
            &serde_json::to_string(&turn::request(s, Vec::new(), true).tools).unwrap_or_default(),
        ),
        detail: format!("{} tools", s.surface.tools.len()),
        zone: Some(ContextZone::CachedPrefix),
        prunable_indices: Vec::new(),
    });
    let mut note = None;
    let mut tail_items = Vec::new();
    match history {
        None => note = Some("a turn is running; the conversation is shown between turns".into()),
        Some(history) => {
            struct GroupAcc {
                name: &'static str,
                tokens: u32,
                count: usize,
                pruned: usize,
                prunable_indices: Vec<usize>,
                all_cached: bool,
            }
            impl GroupAcc {
                fn new(name: &'static str) -> Self {
                    Self {
                        name,
                        tokens: 0,
                        count: 0,
                        pruned: 0,
                        prunable_indices: Vec::new(),
                        all_cached: true,
                    }
                }
            }
            let uncached_from = history
                .iter()
                .rposition(|m| matches!(m, Message::User { .. }))
                .unwrap_or(history.len());
            let mut groups = [
                GroupAcc::new("Summary"),
                GroupAcc::new("Your messages"),
                GroupAcc::new("Model replies and tool calls"),
                GroupAcc::new("Tool results"),
            ];
            let native = s.provider.native_compaction();
            for (idx, m) in history.iter().enumerate() {
                let g = match m {
                    _ if compact::is_summary(m) => 0,
                    Message::User { .. } | Message::System { .. } | Message::Compaction { .. } => 1,
                    Message::Assistant { .. } => 2,
                    Message::Tool { .. } => 3,
                };
                let msg_tokens =
                    (f64::from(turn::message_tokens(m, native)) * ratio).round() as u32;
                let pruned = matches!(m, Message::Tool { content, .. } if content.starts_with(prune::MARKER));
                let prunable = matches!(m, Message::Tool { .. }) && !pruned;
                let cached = idx < uncached_from;
                groups[g].tokens += msg_tokens;
                groups[g].count += 1;
                if pruned {
                    groups[g].pruned += 1;
                }
                if prunable {
                    groups[g].prunable_indices.push(idx);
                }
                if !cached {
                    groups[g].all_cached = false;
                }
                tail_items.push(ContextItem {
                    index: idx,
                    role: item_role(m),
                    label: item_label(m),
                    tokens: msg_tokens,
                    pruned,
                    cached,
                    prunable,
                });
            }
            for (i, group) in groups.into_iter().enumerate() {
                if group.count == 0 {
                    continue;
                }
                let detail = match i {
                    0 => "earlier conversation, summarized".to_owned(),
                    3 if group.pruned > 0 => {
                        format!("{} results, {} pruned", group.count, group.pruned)
                    }
                    _ => format!("{} messages", group.count),
                };
                let zone = if i == 0 || group.all_cached {
                    ContextZone::CachedPrefix
                } else {
                    ContextZone::UncachedTail
                };
                segments.push(ContextSegment {
                    name: group.name.into(),
                    tokens: group.tokens,
                    detail,
                    zone: Some(zone),
                    prunable_indices: group.prunable_indices,
                });
            }
        }
    }
    let threshold = threshold(s);
    ContextReport {
        model: s.model_ref.clone(),
        window: s.model.context_window,
        budget: s.budget,
        threshold,
        used: history.map_or(0, |h| turn::estimate(s, h)),
        strategy: format!("{:?}", s.context.cache_strategy).to_lowercase(),
        segments,
        note,
        tail_items,
    }
}

fn item_role(m: &Message) -> String {
    match m {
        _ if compact::is_summary(m) => "summary".into(),
        Message::User { .. } | Message::Compaction { .. } => "user".into(),
        Message::System { .. } => "system".into(),
        Message::Assistant { .. } => "assistant".into(),
        Message::Tool { .. } => "tool".into(),
    }
}

fn item_label(m: &Message) -> String {
    let raw = match m {
        Message::User { content, .. }
        | Message::System { content }
        | Message::Compaction { content, .. } => content.as_str(),
        Message::Assistant {
            content,
            tool_calls,
            ..
        } => {
            if !tool_calls.is_empty() && content.trim().is_empty() {
                return format!(
                    "called {} tool{}",
                    tool_calls.len(),
                    if tool_calls.len() == 1 { "" } else { "s" }
                );
            }
            content.as_str()
        }
        Message::Tool { content, .. } => content.as_str(),
    };
    let line = raw
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("(empty)");
    let mut clipped: String = line.chars().take(88).collect();
    if line.chars().count() > 88 {
        clipped.push('…');
    }
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_ranges() {
        assert_eq!(read_lines(&json!({})), None);
        assert_eq!(read_lines(&json!({"offset": 1})), None);
        assert_eq!(read_lines(&json!({"offset": 10})), Some((10, u64::MAX)));
        assert_eq!(
            read_lines(&json!({"offset": 10, "limit": 5})),
            Some((10, 14))
        );
        assert_eq!(read_lines(&json!({"limit": 5})), Some((1, 5)));
    }
}
