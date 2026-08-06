//! Where an embedding request goes when the configured backend cannot embed.
//!
//! Pure decision, no I/O. Exists because `BackendCapabilities::embeddings` has
//! been declared since the trait was written and **nothing ever read it** — so
//! installing a backend that cannot embed (Apple's Foundation Models framework
//! exposes no embedding API at all) would have surfaced as whatever confusing
//! error that backend happened to return from a method it does not implement,
//! in the middle of retrieval, per query.
//!
//! `docs/DECISIONS.md` ("Embeddings stay local on every iOS tier, via bundled
//! llama.cpp") fixes the policy: retrieval embeddings never leave the device on
//! iOS, on any tier, using the bundled `nomic-embed-text-v1.5` GGUF. So a
//! backend that cannot embed does not disable retrieval — it delegates.

/// Which provider should serve an embedding request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingRoute {
    /// The configured backend embeds for itself.
    Primary,
    /// The configured backend cannot embed; the local embedder does it.
    LocalFallback,
    /// Nothing on this machine can embed. Callers must degrade to keyword
    /// search and say so, rather than failing the whole query.
    Unavailable,
}

impl EmbeddingRoute {
    /// Whether an embedding can be produced at all.
    pub fn is_available(self) -> bool {
        !matches!(self, EmbeddingRoute::Unavailable)
    }
}

/// Decide where an embedding request goes.
///
/// `primary_embeds` is the configured backend's own
/// `capabilities().embeddings`; `local_embedder_available` is whether a local
/// embedding backend could be constructed (embedded llama.cpp with an
/// embedding model on disk).
pub fn plan_embedding_route(primary_embeds: bool, local_embedder_available: bool) -> EmbeddingRoute {
    if primary_embeds {
        // Deliberately first: a backend that embeds for itself is used even
        // when a local embedder exists. Preferring the local one would load a
        // second model into memory for no gain, and — worse — would silently
        // change the vector space of new rows relative to the ones already
        // indexed by the primary.
        return EmbeddingRoute::Primary;
    }
    if local_embedder_available {
        return EmbeddingRoute::LocalFallback;
    }
    EmbeddingRoute::Unavailable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backend_that_embeds_serves_its_own_requests() {
        assert_eq!(plan_embedding_route(true, false), EmbeddingRoute::Primary);
    }

    #[test]
    fn an_available_local_embedder_never_displaces_a_capable_backend() {
        // Mixing vector spaces mid-corpus is worse than any memory saving:
        // rows embedded by two different models are not comparable, and the
        // damage is silent — retrieval just quietly returns the wrong threads.
        assert_eq!(plan_embedding_route(true, true), EmbeddingRoute::Primary);
    }

    #[test]
    fn a_backend_without_embeddings_delegates_locally() {
        // The Foundation Models case: no embedding API exists in the framework,
        // so retrieval runs on the bundled GGUF instead of being switched off.
        assert_eq!(plan_embedding_route(false, true), EmbeddingRoute::LocalFallback);
    }

    #[test]
    fn no_embedder_anywhere_is_reported_rather_than_guessed() {
        assert_eq!(plan_embedding_route(false, false), EmbeddingRoute::Unavailable);
    }

    #[test]
    fn availability_is_the_question_callers_actually_ask() {
        assert!(plan_embedding_route(true, false).is_available());
        assert!(plan_embedding_route(false, true).is_available());
        assert!(!plan_embedding_route(false, false).is_available());
    }
}
