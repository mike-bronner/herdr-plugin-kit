//! What a caller may name as the result of a call.
//!
//! Hand-written, and it says nothing the schema decides: no variant, no tag,
//! and no field. The implementations are generated, one per branch of
//! `success_response`, so a Herdr release that adds a result type needs
//! nothing here. See `codegen/split_results.py`.
//!
//! # Why naming one type is worth a trait
//!
//! [`generated::ResponseResult`](crate::api::generated::ResponseResult) is one
//! enum carrying all 64 shapes Herdr can answer with. Deserializing it costs a
//! binary every one of them: serde generates parsing code per variant, and
//! dead-code elimination cannot drop any, because each is reachable through
//! the single type.
//!
//! ✅ **Measured 2026-09-11 by the recent-spaces migration**, on macOS arm64 at
//! `opt-level = "s"` with `strip = true`. Deserializing the union accounts for
//! **1,951,648 bytes**, against 102,368 for serializing all 102 request
//! methods. That plugin reads one variant of the 64.
//!
//! So every variant also has a type of its own, and a caller names the one it
//! expects. The other 63 are then never instantiated, and the linker drops
//! them.
//!
//! # The union is still here, and still callable
//!
//! ⚠️ **A narrower option, not a replacement.** A caller that genuinely wants
//! any response Herdr can send names `ResponseResult`, which implements this
//! trait too, and pays for all 64 knowingly rather than by default.
//!
//! # Naming the wrong type is refused, not absorbed
//!
//! 🚨 **The schema does not say what a method answers**, and the obvious guess
//! is wrong on the first plugin that looked: `workspace.move` answers
//! `workspace_list`, carrying the sidebar after the move, and `workspace_moved`
//! is not a result type at all. So a plugin author establishes the pairing by
//! measurement whatever this kit does, and a wrong guess has to fail loudly.
//!
//! Every generated result type carries its own `type` tag as a one-variant
//! enum, so an answer meant for another variant fails to deserialize and
//! becomes [`CallError::Protocol`](crate::api::client::CallError::Protocol).
//! 🔑 That check lives in the type rather than in the function that calls it,
//! which matters most for the narrowest type in the set: `OkAnswer` declares
//! nothing but its tag, and without the check it would accept every object
//! Herdr can send.

use serde::de::DeserializeOwned;

/// A result shape a caller can ask for by name.
///
/// Implemented by one generated type per response variant, and by
/// [`generated::ResponseResult`](crate::api::generated::ResponseResult) for a
/// caller that wants any of them.
///
/// ⚠️ **Nothing to implement, and nothing worth implementing.** It carries no
/// method on purpose: the tag check that makes a narrow type safe is inside
/// the generated type's own `Deserialize`, not in anything this trait could
/// declare. A type the pipeline did not generate can satisfy it, and would
/// then be a hand-written description of Herdr's wire format, which is the one
/// thing this crate exists to end.
pub trait ResponseVariant: DeserializeOwned {}
