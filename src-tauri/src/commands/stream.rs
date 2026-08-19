use tauri::{State, WebviewWindow, ipc::Response};

use crate::{
    channels::stream_runtime::validate_subscription_id,
    contract::{
        CommandError,
        stream::{StreamAckReceipt, StreamKind},
    },
    state::{AppState, StreamDeliveryError},
};

#[tauri::command]
pub(crate) fn read_stream_delivery(
    stream_kind: String,
    subscription_id: String,
    stream_generation: String,
    delivery_sequence: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<Response, CommandError> {
    let stream_kind = parse_stream_kind(&stream_kind)?;
    validate_subscription_id(&subscription_id)?;
    let stream_generation = parse_counter(&stream_generation)?;
    let delivery_sequence = parse_counter(&delivery_sequence)?;
    let bytes = state
        .take_stream_delivery(
            window.label(),
            &stream_kind.stream_key(&subscription_id),
            stream_generation,
            delivery_sequence,
        )
        .map_err(stream_delivery_error)?;
    Ok(Response::new(bytes))
}

#[tauri::command]
pub(crate) fn ack_stream_delivery(
    stream_kind: String,
    subscription_id: String,
    stream_generation: String,
    delivery_sequence: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<StreamAckReceipt, CommandError> {
    let stream_kind = parse_stream_kind(&stream_kind)?;
    validate_subscription_id(&subscription_id)?;
    let stream_generation = parse_counter(&stream_generation)?;
    let delivery_sequence = parse_counter(&delivery_sequence)?;
    let accepted = state
        .ack_stream_delivery(
            window.label(),
            &stream_kind.stream_key(&subscription_id),
            stream_generation,
            delivery_sequence,
        )
        .map_err(stream_delivery_error)?;
    Ok(StreamAckReceipt { accepted })
}

fn parse_stream_kind(value: &str) -> Result<StreamKind, CommandError> {
    StreamKind::parse(value).ok_or_else(CommandError::invalid_stream_delivery)
}

fn parse_counter(value: &str) -> Result<u64, CommandError> {
    if value.is_empty()
        || value.len() > 20
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(CommandError::invalid_stream_delivery());
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(CommandError::invalid_stream_delivery)
}

fn stream_delivery_error(error: StreamDeliveryError) -> CommandError {
    match error {
        StreamDeliveryError::InvalidRequest => CommandError::invalid_stream_delivery(),
        StreamDeliveryError::PayloadTooLarge => CommandError::stream_delivery_too_large(),
        StreamDeliveryError::RuntimeUnavailable => CommandError::stream_runtime_unavailable(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_identity_accepts_only_exact_stable_codes_and_positive_decimal_counters() {
        assert_eq!(
            parse_stream_kind("mainDps").expect("stable stream kind"),
            StreamKind::MainDps
        );
        assert!(parse_stream_kind("main-dps").is_err());
        assert_eq!(
            parse_counter("18446744073709551615").expect("maximum decimal counter"),
            u64::MAX
        );
        assert!(parse_counter("0").is_err());
        assert!(parse_counter("-1").is_err());
        assert!(parse_counter("01").is_err());
        assert!(parse_counter("01x").is_err());
    }

    #[test]
    fn delivery_errors_are_stable_and_do_not_include_request_identity() {
        for error in [
            StreamDeliveryError::InvalidRequest,
            StreamDeliveryError::PayloadTooLarge,
            StreamDeliveryError::RuntimeUnavailable,
        ] {
            let mapped = stream_delivery_error(error);
            assert!(mapped.message_arguments.is_empty());
            assert!(mapped.diagnostic_line.is_none());
        }
    }
}
