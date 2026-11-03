//! The parse half of Swift's `QuestionAdapter` (+Agy): question cards from tool calls, and their state from results.
//! One implementation, in `question_adapter`; the parser reaches it through these names.

pub use crate::question_adapter::{
    agy, agy_state, answered, claude, claude_state, codex, codex_state, grok, grok_state, strip_recommended, CODEX_OTHER_LABEL,
    CODEX_TOOLS, RECOMMENDED_SUFFIXES,
};
