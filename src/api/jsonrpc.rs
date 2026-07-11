use serde::Serialize;
use serde_json::{Map, Value};

use super::request::{Request, parse_request};

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
pub const DOMAIN_ERROR: i64 = -32000;

#[derive(Debug)]
pub struct ValidatedRequest {
    pub id: Value,
    pub request: Request,
}

#[derive(Debug)]
pub struct RequestError {
    pub id: Value,
    pub error: RpcError,
}

#[derive(Debug)]
pub struct RpcError {
    pub code: i64,
    pub message: &'static str,
    pub data: Option<ErrorData>,
}

#[derive(Debug, Serialize)]
pub struct ErrorData {
    pub domain_code: &'static str,
    pub detail: String,
}

impl RpcError {
    pub fn parse_error() -> Self {
        Self {
            code: PARSE_ERROR,
            message: "Parse error",
            data: None,
        }
    }

    pub fn invalid_request() -> Self {
        Self {
            code: INVALID_REQUEST,
            message: "Invalid Request",
            data: None,
        }
    }

    pub fn method_not_found() -> Self {
        Self {
            code: METHOD_NOT_FOUND,
            message: "Method not found",
            data: None,
        }
    }

    pub fn invalid_params(detail: impl Into<String>) -> Self {
        Self {
            code: INVALID_PARAMS,
            message: "Invalid params",
            data: Some(ErrorData {
                domain_code: "INVALID_PARAMS",
                detail: detail.into(),
            }),
        }
    }

    pub fn domain(domain_code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code: DOMAIN_ERROR,
            message: "Core error",
            data: Some(ErrorData {
                domain_code,
                detail: detail.into(),
            }),
        }
    }

    pub fn internal() -> Self {
        Self {
            code: INTERNAL_ERROR,
            message: "Internal error",
            data: None,
        }
    }
}

pub fn parse_line(line: &str) -> Result<ValidatedRequest, RequestError> {
    let value: Value = serde_json::from_str(line)
        .map_err(|_| request_error(Value::Null, RpcError::parse_error()))?;
    let object = value
        .as_object()
        .ok_or_else(|| request_error(Value::Null, RpcError::invalid_request()))?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(request_error(Value::Null, RpcError::invalid_request()));
    }
    let id = object
        .get("id")
        .filter(|id| valid_request_id(id))
        .cloned()
        .ok_or_else(|| request_error(Value::Null, RpcError::invalid_request()))?;
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| request_error(Value::Null, RpcError::invalid_request()))?;
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    let request =
        parse_request(method, params).map_err(|error| request_error(id.clone(), error))?;
    Ok(ValidatedRequest { id, request })
}

fn request_error(id: Value, error: RpcError) -> RequestError {
    RequestError { id, error }
}

fn valid_request_id(id: &Value) -> bool {
    match id {
        Value::Null | Value::String(_) => true,
        Value::Number(number) => number.is_i64() || number.is_u64(),
        _ => false,
    }
}

pub fn success<T: Serialize>(id: Value, result: T) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

pub fn failure(id: Value, error: RpcError) -> Value {
    let mut body = Map::new();
    body.insert("code".to_owned(), Value::from(error.code));
    body.insert("message".to_owned(), Value::from(error.message));
    if let Some(data) = error.data {
        body.insert(
            "data".to_owned(),
            serde_json::to_value(data).expect("JSON-RPC error data must serialize"),
        );
    }
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": body,
    })
}

pub fn failure_without_id(error: RpcError) -> Value {
    failure(Value::Null, error)
}

pub fn notification<T: Serialize>(method: &'static str, params: T) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_batch_and_invalid_ids() {
        assert_eq!(parse_line("[]").unwrap_err().error.code, INVALID_REQUEST);
        assert_eq!(
            parse_line(r#"{"jsonrpc":"2.0","id":true,"method":"core.status"}"#)
                .unwrap_err()
                .error
                .code,
            INVALID_REQUEST
        );
    }

    #[test]
    fn preserves_string_and_numeric_ids() {
        let string =
            parse_line(r#"{"jsonrpc":"2.0","id":"request-1","method":"core.status"}"#).unwrap();
        let numeric = parse_line(r#"{"jsonrpc":"2.0","id":7,"method":"core.status"}"#).unwrap();
        assert_eq!(string.id, "request-1");
        assert_eq!(numeric.id, 7);
    }

    #[test]
    fn invalid_params_preserve_a_valid_request_id() {
        let error =
            parse_line(r#"{"jsonrpc":"2.0","id":"bad-params","method":"core.status","params":[]}"#)
                .unwrap_err();
        assert_eq!(error.id, "bad-params");
        assert_eq!(error.error.code, INVALID_PARAMS);
    }
}
