//! Route table and per-request handling.
//!
//! One function, in one place, decides what a request is allowed to do. The order of the checks
//! is deliberate and is itself a control:
//!
//! 1. **Method.** Only `GET`. A read-only API that never routes a `POST` cannot be tricked into
//!    a state change by a forged form submission, whatever else goes wrong.
//! 2. **No request body.** Nothing here reads one, so a request carrying one is refused rather
//!    than having megabytes streamed into the process to be discarded.
//! 3. **Request target.** Bounded and strictly decoded, before anything looks at it.
//! 4. **Guard.** `Host`, `Origin` and the session token (THREAT-MODEL T4). This runs before
//!    routing, so an unauthorised caller cannot even learn which routes exist, and before any
//!    database work, so it cannot make an unauthorised server do work.
//! 5. **Route.** Exact match against a fixed table.
//!
//! Every response, including every refusal, carries the security headers.

use tiny_http::{Method, Request};

use crate::api::{self, ApiError, Context};
use crate::assets;
use crate::guard::{Guard, Rejection};
use crate::request::{Target, TargetError};
use crate::respond;
use crate::token::{TOKEN_HEADER, TOKEN_QUERY};

/// Header inspected for the request's authority.
const HOST_HEADER: &str = "Host";
/// Header inspected for the requesting document's origin.
const ORIGIN_HEADER: &str = "Origin";

/// What a request resolved to.
enum Outcome {
    /// A JSON body with a status.
    Json(u16, serde_json::Value),
    /// A refusal, in the standard error shape.
    Error(ApiError),
    /// An embedded asset.
    Asset(&'static assets::Asset),
}

fn header<'a>(request: &'a Request, field: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv(field))
        .map(|header| header.value.as_str())
}

/// Handle one request end to end and write the response.
pub fn handle(request: Request, guard: &Guard, ctx: &Context<'_>) {
    let outcome = resolve(&request, guard, ctx);
    let response = match outcome {
        Outcome::Json(status, value) => respond::json(status, &value),
        Outcome::Error(error) => respond::error(
            error.status,
            error.code,
            &error.message,
            error.detail.clone(),
        ),
        Outcome::Asset(asset) => respond::asset(asset.content_type, asset.bytes),
    };
    // A client that hung up mid-response is not an error condition worth reporting: there is
    // nobody left to report it to, and this server has no log to write it into.
    let _ = request.respond(response);
}

fn resolve(request: &Request, guard: &Guard, ctx: &Context<'_>) -> Outcome {
    if *request.method() != Method::Get {
        return Outcome::Error(ApiError::new(
            405,
            "method_not_allowed",
            "this API is read-only; only GET is served",
        ));
    }
    if request.body_length().unwrap_or(0) > 0 {
        return Outcome::Error(ApiError::new(
            413,
            "body_not_accepted",
            "no endpoint accepts a request body",
        ));
    }

    let target = match Target::parse(request.url()) {
        Ok(target) => target,
        Err(TargetError::TooLong) => {
            return Outcome::Error(ApiError::new(
                414,
                "target_too_long",
                "request target exceeds the accepted length",
            ))
        }
        Err(TargetError::TooManyParameters) => {
            return Outcome::Error(ApiError::new(
                400,
                "too_many_parameters",
                "request carries more query parameters than are accepted",
            ))
        }
        Err(TargetError::Malformed) => {
            return Outcome::Error(ApiError::new(
                400,
                "malformed_target",
                "request target is not valid percent-encoded UTF-8",
            ))
        }
    };

    let supplied_token = header(request, TOKEN_HEADER)
        .map(str::to_string)
        .or_else(|| target.get(TOKEN_QUERY).map(str::to_string));
    if let Err(rejection) = guard.check(
        header(request, HOST_HEADER),
        header(request, ORIGIN_HEADER),
        supplied_token.as_deref(),
    ) {
        // A browser cannot attach a header to a subresource request. The document is opened at
        // `/?token=…`, but the `<script>` and `<link>` it names are fetched by the browser with
        // no token and no way to supply one — so requiring the token on the embedded assets makes
        // the interface unloadable. It is relaxed for those, and for nothing else:
        //
        //  * `Host` and `Origin` are still enforced. [`Guard::check`] applies them **before** the
        //    token, so a `MissingToken` or `BadToken` verdict is proof that both already passed —
        //    the DNS-rebinding defence and the cross-origin refusal are untouched.
        //  * Only a path that resolves in the fixed asset table is served. Anything else still
        //    gets the guard's refusal, so an unauthorised caller still cannot learn which API
        //    routes exist.
        //  * What is served is build-constant: the same bytes in every copy of this binary,
        //    containing no repository content, no index content and no session state. A caller
        //    who can reach these already has the executable they came out of.
        //
        // Every `/api/*` route remains gated on all three checks.
        let unauthenticated_asset =
            matches!(rejection, Rejection::MissingToken | Rejection::BadToken)
                && assets::lookup(&target.path).is_some();

        if !unauthenticated_asset {
            return Outcome::Error(ApiError::new(
                rejection.status(),
                rejection.code(),
                rejection.message(),
            ));
        }
    }

    route(&target, ctx)
}

fn route(target: &Target, ctx: &Context<'_>) -> Outcome {
    let answer = match target.path.as_str() {
        "/api/overview" => api::overview(ctx),
        "/api/search" => api::search(ctx, target),
        "/api/entity" => api::entity(ctx, target),
        "/api/neighbourhood" => api::neighbourhood(ctx, target),
        "/api/path" => api::path(ctx, target),
        "/api/why" => api::why(ctx, target),
        "/api/source" => api::source(ctx, target),
        "/api/unresolved" => api::unresolved(ctx, target),
        "/api/partial-parses" => api::partial_parses(ctx),
        other => {
            return match assets::lookup(other) {
                Some(asset) => Outcome::Asset(asset),
                None => Outcome::Error(ApiError::with_detail(
                    404,
                    "no_such_route",
                    "no such route",
                    serde_json::json!({ "path": other, "routes": ROUTES }),
                )),
            }
        }
    };
    match answer {
        Ok(value) => Outcome::Json(200, wrap(value)),
        Err(error) => Outcome::Error(error),
    }
}

/// Every JSON answer carries the same envelope, so a client can branch on one field.
fn wrap(value: serde_json::Value) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    object.insert("ok".into(), serde_json::json!(true));
    match value {
        serde_json::Value::Object(fields) => {
            for (key, field) in fields {
                object.insert(key, field);
            }
        }
        other => {
            object.insert("data".into(), other);
        }
    }
    serde_json::Value::Object(object)
}

/// The routes this build serves, in the order a client would meet them.
pub const ROUTES: [&str; 9] = [
    "/api/overview",
    "/api/search",
    "/api/entity",
    "/api/neighbourhood",
    "/api/path",
    "/api/why",
    "/api/source",
    "/api/unresolved",
    "/api/partial-parses",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_envelope_merges_rather_than_nests() {
        let wrapped = wrap(serde_json::json!({ "count": 2 }));
        assert_eq!(wrapped["ok"], true);
        assert_eq!(wrapped["count"], 2);
    }

    #[test]
    fn a_non_object_answer_is_carried_under_data() {
        let wrapped = wrap(serde_json::json!([1, 2, 3]));
        assert_eq!(wrapped["ok"], true);
        assert_eq!(wrapped["data"], serde_json::json!([1, 2, 3]));
    }

    #[test]
    fn every_advertised_route_is_under_the_api_prefix() {
        for route in ROUTES {
            assert!(route.starts_with("/api/"), "{route}");
        }
    }
}
