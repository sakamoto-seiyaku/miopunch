//! Startup RTC capability, separate from resource permission and reachability.
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct RtcCapability {
    /// None means policy prevented the runtime check, not that support failed.
    pub supported: Option<bool>,
    pub permitted: bool,
    pub effective: bool,
    pub reason: CapabilityReason,
    pub probe: ProbeReceipt,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CapabilityReason {
    DisabledByPolicy,
    FeatureDisabled,
    ApiMissing,
    PeerConnectionRejected,
    DataChannelRejected,
    ChannelConfigurationUnsupported,
    CleanupFailed,
    Available,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProbeReceipt {
    pub pc_created: bool,
    pub channel_created: bool,
    pub pc_closed: bool,
    pub channel_close_requested: bool,
}

impl RtcCapability {
    fn without_probe(permitted: bool, supported: Option<bool>, reason: CapabilityReason) -> Self {
        Self {
            supported,
            permitted,
            effective: false,
            reason,
            probe: ProbeReceipt {
                pc_created: false,
                channel_created: false,
                pc_closed: true,
                channel_close_requested: false,
            },
        }
    }

    /// The real browser adapter runs synchronously without SDP or ICE gathering.
    /// Policy is checked before calling any browser RTC constructor or getter.
    pub(crate) fn probe(permitted: bool) -> Result<Self, JsValue> {
        if !permitted {
            return Ok(Self::without_probe(
                false,
                None,
                CapabilityReason::DisabledByPolicy,
            ));
        }
        #[cfg(all(target_arch = "wasm32", feature = "rtc-probe"))]
        {
            let raw = probe_rtc_capability(
                true,
                crate::rtc_association::DATAGRAM_CHANNEL_LABEL,
                crate::rtc_association::DATAGRAM_CHANNEL_PROTOCOL,
            )
            .map_err(|_| JsValue::from_str("RTC runtime capability check failed"))?;
            let value: Self = serde_json::from_str(&raw)
                .map_err(|_| JsValue::from_str("invalid RTC capability receipt"))?;
            if value.probe.pc_created && !value.probe.pc_closed {
                return Err(JsValue::from_str("RTC capability probe did not close"));
            }
            Ok(value)
        }
        #[cfg(not(all(target_arch = "wasm32", feature = "rtc-probe")))]
        {
            Ok(Self::without_probe(
                true,
                Some(false),
                CapabilityReason::FeatureDisabled,
            ))
        }
    }
}

#[cfg(all(target_arch = "wasm32", feature = "rtc-probe"))]
#[wasm_bindgen(module = "/src/rtc_capability.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = probeRtcCapability)]
    fn probe_rtc_capability(
        permitted: bool,
        label: &str,
        protocol: &str,
    ) -> Result<String, JsValue>;
}
