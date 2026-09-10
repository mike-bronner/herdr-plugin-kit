//! The one hand-written type in the generated layer.
//!
//! Herdr's request schema puts an `id` property at the top level, beside the
//! `oneOf` that lists all 102 methods. `cargo-typify` 0.8.0 has to merge that
//! sibling property into every branch, and in that merge path it abandons the
//! serde discriminator: the enum comes out `#[serde(untagged)]` as
//! `Variant0`..`Variant101`, `method` becomes an unconstrained `String`, and
//! every method deserializes to `Variant0`.
//!
//! That failure is silent. The broken types still round-trip JSON perfectly,
//! because the method name rides along as an opaque string, and they fail open
//! by accepting invented methods and missing required params.
//!
//! So the pipeline removes `id` before generation, and this file puts it back
//! once, around the outside. See `codegen/lift_envelope.py`.

use serde::{Deserialize, Serialize};

use super::generated::RequestMethod;

/// A request on Herdr's wire protocol: a correlation id, and a method.
///
/// This is the only type in the generated layer written by hand, and it stays
/// that way because of what it does *not* say. It names no method, no variant,
/// and no params shape, so a regeneration that adds or renames a method never
/// touches it. Anything added here that names one of those would have to be
/// re-checked against every future Herdr release.
///
/// # Unknown fields
///
/// `#[serde(flatten)]` buffers through a map, so `deny_unknown_fields` does
/// not work through this wrapper and unknown top-level keys are ignored. That
/// is a serde limitation rather than a choice. The method discriminator itself
/// is still closed: an unrecognised `method` is rejected.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Request {
    /// Correlates this request with the response that answers it.
    pub id: String,

    /// The method, and the params it carries.
    ///
    /// Flattened, so the wire form is `{"id": ..., "method": ..., "params": ...}`
    /// rather than a nested object. That is what Herdr sends and expects.
    #[serde(flatten)]
    pub method: RequestMethod,
}
