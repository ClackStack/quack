# Work modes and approvals

quack has two independent controls: **Plan / Edit** for what the agent can do, and
**Ask / Allow** for when it needs approval. New sessions default to **Edit + Ask**.

| Work mode | Ask | Allow |
|---|---|---|
| **Plan** | Read and search; unapproved tools ask | Read and search automatically |
| **Edit** | Read and search; unapproved edits and commands ask | Read, search, edit and run commands automatically |

**Plan + Allow** lets the agent investigate without approving each eligible action.
It keeps the configured sandbox. Explicit ask rules, deny rules and approval for leaving
the sandbox still apply in Allow.

## Plan first, then implement

Plan reads and searches, asks clarifying questions, and presents proposed changes plus
the exact test commands it wants to run and what each verifies. If no tests apply, it
should explain why. Test steps stay pending for your review.

Plan refuses file changes, memory writes, tests, builds, arbitrary scripts, and mutating
MCP tools or sub-agents. Allow rules, remembered approvals and yolo cannot override
these restrictions. MCP tools are eligible only when their server marks them read-only.

After a planning turn, choose **Implement plan** in the review bar or run `/plan accept`.
This switches to Edit and starts implementation with the same approvals. **Keep planning**
lets you revise the plan. `/edit` or `/plan off` leaves planning without starting a turn;
`/edit <task>` switches to Edit and submits that task.

## Controls

The TUI shows work mode and approvals separately.

| Control | Effect |
|---|---|
| **Shift-Tab** | Toggle Plan/Edit, keeping approvals |
| `/plan [task]` | Enter Plan, optionally submitting a task |
| `/edit [task]` | Enter Edit, optionally submitting a task |
| `/plan accept` | Implement the plan, keeping approvals |
| `/permissions` | Pick Ask/Allow and inspect permission rules |
| `/permissions ask` | Ask before unapproved actions, keeping work mode |
| `/permissions allow` | Automatically approve eligible actions, keeping work mode |

For example, enter `/permissions allow`, then `/plan make the parser faster`.
The agent investigates and proposes tests for you to review before implementation.

The CLI exposes the same controls:

```sh
quack --mode plan --approvals allow
quack --mode edit --approvals ask
quack exec --mode plan --approvals allow "Plan a fix for the date parser"
```

Headless runs cannot answer approval prompts: actions that still require approval are
denied with feedback to the model. Use scoped allow rules or `--approvals allow` as
appropriate. Plan restrictions still apply.

In `config.toml`:

```toml
[permissions]
work_mode = "plan"  # plan or edit; default: edit
mode = "allow"      # ask or allow; default: ask
```

Both selections survive reload and session resume, subject to current system policy.
ACP editors expose separate Plan/Edit and Ask/Allow controls. Administrators can cap
approvals with `policy.max_permission_mode` and separately lock
`permissions.work_mode` to Plan; a Plan lock also prevents plan acceptance.

## Yolo and the sandbox

`/yolo` toggles automatic approvals with the sandbox off; `/yolo off` restores the
previous approval policy. `--yolo` starts a run with that bypass. Work mode is preserved,
so **Plan + Yolo** still refuses changes and test execution. Deny rules and the built-in
safety list always apply.

**Allow** keeps the configured sandbox. `/sandbox` controls sandboxing separately.
See [security](plan/11-security.md) for platform support, rules and enforcement details.

## Older settings

The standard choices are now Ask and Allow. Existing configurations and saved sessions
using `auto-edit` or `read-only` keep their behavior; these values are hidden from normal
choices and remain visible while active. They are never silently converted to a more
permissive setting.

- For read/search-only work, choose **Plan** with your preferred approvals.
- For automatic edits in selected files, choose **Ask** and add a scoped allow rule,
  such as `allow = ["edit:src/**"]` under `[permissions]`.
- `full-auto` remains an alias for `allow`, and `--permission-mode` remains an alias for
  `--approvals`.

The sandbox's separate `read-only` setting and sub-agents' `read_only` flag remain
available; they control different restrictions.
