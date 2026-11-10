//! The 16 ms delta coalescer (docs/plan/15-desktop-client.md#the-bridge).
//!
//! Engine streams can produce hundreds of [`Event::AssistantDelta`],
//! [`Event::ReasoningDelta`] and [`Event::ToolCallOutputDelta`] events per second. The
//! webview sink (`Channel::send` on desktop) never blocks, so sending each one means
//! IPC and DOM churn that outpaces rendering. The coalescer buffers deltas and merges
//! chunks addressed at the same stream; the registry flushes them on a 16 ms tick and
//! always flushes before a non-delta event, so ordering with the rest of the protocol
//! is preserved. Deltas inside [`Event::Agent`] are buffered per sub-agent the same way.

use std::collections::HashMap;

use quack_protocol::{CallId, Event};

/// One buffered output stream: streaming text or reasoning of a step, or the output of
/// one tool call, either in the main turn (`None`) or inside a sub-agent (`Some`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Stream {
    Assistant(Option<String>),
    Reasoning(Option<String>),
    ToolOutput(Option<String>, CallId),
}

/// Buffers delta events; non-delta events pass straight through.
#[derive(Debug, Default)]
pub struct DeltaCoalescer {
    /// Insertion-ordered, so flushed events replay in the order the streams started.
    buffers: Vec<(Stream, String)>,
    index: HashMap<Stream, usize>,
}

impl DeltaCoalescer {
    /// Feed one event. `None` means it was a delta and is buffered — send nothing yet.
    /// `Some(event)` means send `flush()` first if anything is buffered, then the event.
    pub fn on(&mut self, event: &Event) -> bool {
        match event {
            Event::AssistantDelta { text } => self.push(Stream::Assistant(None), text.clone()),
            Event::ReasoningDelta { text } => self.push(Stream::Reasoning(None), text.clone()),
            Event::ToolCallOutputDelta { call_id, chunk } => {
                self.push(Stream::ToolOutput(None, call_id.clone()), chunk.clone())
            }
            Event::Agent { id, inner } => match inner.as_ref() {
                Event::AssistantDelta { text } => {
                    self.push(Stream::Assistant(Some(id.clone())), text.clone())
                }
                Event::ReasoningDelta { text } => {
                    self.push(Stream::Reasoning(Some(id.clone())), text.clone())
                }
                Event::ToolCallOutputDelta { call_id, chunk } => self.push(
                    Stream::ToolOutput(Some(id.clone()), call_id.clone()),
                    chunk.clone(),
                ),
                _ => true,
            },
            _ => true,
        }
    }

    /// Append `text` to `stream`'s buffer; `false` (buffered).
    fn push(&mut self, stream: Stream, text: String) -> bool {
        match self.index.get(&stream) {
            Some(&i) => self.buffers[i].1.push_str(&text),
            None => {
                self.index.insert(stream.clone(), self.buffers.len());
                self.buffers.push((stream, text));
            }
        }
        false
    }

    /// The buffered deltas as protocol events, merged per stream, oldest stream first.
    /// Buffering stops; the next delta starts a fresh buffer.
    pub fn flush(&mut self) -> Vec<Event> {
        let buffers = std::mem::take(&mut self.buffers);
        self.index.clear();
        buffers
            .into_iter()
            .map(|(stream, text)| match stream {
                Stream::Assistant(agent) => wrap(agent, Event::AssistantDelta { text }),
                Stream::Reasoning(agent) => wrap(agent, Event::ReasoningDelta { text }),
                Stream::ToolOutput(agent, call_id) => wrap(
                    agent,
                    Event::ToolCallOutputDelta {
                        call_id,
                        chunk: text,
                    },
                ),
            })
            .collect()
    }
}

/// Put back the `Event::Agent` wrapper the stream came from.
fn wrap(agent: Option<String>, event: Event) -> Event {
    match agent {
        Some(id) => Event::Agent {
            id,
            inner: Box::new(event),
        },
        None => event,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deltas(texts: &[&str]) -> Vec<Event> {
        texts
            .iter()
            .map(|t| Event::AssistantDelta { text: (*t).into() })
            .collect()
    }

    #[test]
    fn merges_consecutive_chunks_of_one_stream() {
        let mut c = DeltaCoalescer::default();
        assert!(!c.on(&deltas(&["a"])[0]));
        assert!(!c.on(&deltas(&["b"])[0]));
        assert!(!c.on(&deltas(&["c"])[0]));
        assert_eq!(
            c.flush(),
            vec![Event::AssistantDelta { text: "abc".into() }]
        );
        assert!(c.flush().is_empty());
    }

    #[test]
    fn keeps_streams_apart_and_in_start_order() {
        let tool = Event::ToolCallOutputDelta {
            call_id: CallId("t1".into()),
            chunk: "1".into(),
        };
        let mut c = DeltaCoalescer::default();
        assert!(!c.on(&deltas(&["a"])[0]));
        assert!(!c.on(&tool));
        assert!(!c.on(&deltas(&["b"])[0]));
        assert!(!c.on(&Event::ToolCallOutputDelta {
            call_id: CallId("t1".into()),
            chunk: "2".into()
        }));
        assert_eq!(
            c.flush(),
            vec![
                Event::AssistantDelta { text: "ab".into() },
                Event::ToolCallOutputDelta {
                    call_id: CallId("t1".into()),
                    chunk: "12".into()
                },
            ]
        );
    }

    #[test]
    fn non_delta_passes_through_after_a_flush() {
        let mut c = DeltaCoalescer::default();
        assert!(!c.on(&deltas(&["a"])[0]));
        assert!(c.on(&Event::ToolCallBegin {
            call: quack_protocol::ToolCallInfo {
                call_id: CallId("t1".into()),
                name: "shell".into(),
                summary: "shell echo hi".into(),
            },
        }));
        assert_eq!(c.flush(), vec![Event::AssistantDelta { text: "a".into() }]);
    }

    #[test]
    fn agent_deltas_buffer_under_their_own_id() {
        let mut c = DeltaCoalescer::default();
        let inner = || Event::AssistantDelta { text: "x".into() };
        assert!(!c.on(&Event::Agent {
            id: "explore-1".into(),
            inner: Box::new(inner())
        }));
        assert!(!c.on(&Event::Agent {
            id: "explore-1".into(),
            inner: Box::new(inner())
        }));
        assert!(c.on(&Event::Agent {
            id: "explore-1".into(),
            inner: Box::new(Event::AgentFinished {
                id: "explore-1".into(),
                ok: true,
                summary: "done".into()
            }),
        }));
        assert_eq!(
            c.flush(),
            vec![Event::Agent {
                id: "explore-1".into(),
                inner: Box::new(Event::AssistantDelta { text: "xx".into() }),
            }]
        );
    }
}
