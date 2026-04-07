/// Attribution context for every mutation (fact, rule, retraction).
/// Flows from CLI args / HTTP headers → Brain Tx metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationContext {
    pub actor: String,
    pub session: Option<String>,
    pub model: Option<String>,
}

impl Default for MutationContext {
    fn default() -> Self {
        Self {
            actor: "unknown".into(),
            session: None,
            model: None,
        }
    }
}
