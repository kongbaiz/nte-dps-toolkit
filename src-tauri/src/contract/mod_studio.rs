use super::CommandError;
use nte_dps_tool::core::mod_market::{
    ModMarketCatalog, ModMarketError, ModMarketErrorCode, ModMarketItem, ModMarketLocalizations,
};
use serde::Serialize;
pub(crate) const MOD_MARKET_CONTRACT_VERSION: u32 = 13;
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModMarketCatalogSnapshot {
    pub contract_version: u32,
    pub published_at: String,
    pub privacy_mode: &'static str,
    pub mods: Vec<ModMarketItemSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModMarketItemSnapshot {
    pub component: nte_dps_tool::core::mod_market::ComponentKind,
    pub id: String,
    pub bindings: Vec<String>,
    pub localizations: ModMarketLocalizationsSnapshot,
    pub version: String,
    pub author: String,
    pub capabilities: Vec<String>,
    pub package_size: u64,
    pub local_state: ModMarketLocalStateSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum ModMarketLocalStateSnapshot {
    NotInstalled,
    Installed {
        enabled: Option<bool>,
        current: bool,
    },
    Unreadable {
        code: &'static str,
        message_key: &'static str,
    },
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ModMarketLocalizationsSnapshot {
    pub en: ModMarketLocalizedTextSnapshot,
    #[serde(rename = "zh-CN")]
    pub zh_cn: ModMarketLocalizedTextSnapshot,
    pub ja: ModMarketLocalizedTextSnapshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModMarketLocalizedTextSnapshot {
    pub name: String,
    pub summary: String,
}

impl From<ModMarketLocalizations> for ModMarketLocalizationsSnapshot {
    fn from(localizations: ModMarketLocalizations) -> Self {
        Self {
            en: ModMarketLocalizedTextSnapshot {
                name: localizations.english.name,
                summary: localizations.english.summary,
            },
            zh_cn: ModMarketLocalizedTextSnapshot {
                name: localizations.simplified_chinese.name,
                summary: localizations.simplified_chinese.summary,
            },
            ja: ModMarketLocalizedTextSnapshot {
                name: localizations.japanese.name,
                summary: localizations.japanese.summary,
            },
        }
    }
}

impl ModMarketCatalogSnapshot {
    pub(crate) fn from_catalog(
        catalog: ModMarketCatalog,
        local_status: impl Fn(&ModMarketItem) -> ModMarketLocalStateSnapshot,
    ) -> Self {
        Self {
            contract_version: MOD_MARKET_CONTRACT_VERSION,
            published_at: catalog.published_at,
            privacy_mode: "anonymous-read-only",
            mods: catalog
                .mods
                .into_iter()
                .map(|item| {
                    let local_state = local_status(&item);
                    ModMarketItemSnapshot {
                        component: item.component,
                        id: item.id,
                        bindings: item.bindings,
                        localizations: item.localizations.into(),
                        version: item.version.to_string(),
                        author: item.author,
                        capabilities: item.capabilities,
                        package_size: item.package_size,
                        local_state,
                    }
                })
                .collect(),
        }
    }
}

impl CommandError {
    pub(crate) fn mod_studio_risk_acknowledgement_required() -> Self {
        Self {
            code: "mod_studio_risk_acknowledgement_required",
            message_key: "Confirm the Mod risk warning before starting the game runtime.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn from_mod_market(error: ModMarketError) -> Self {
        let (code, message_key) = match error.code {
            ModMarketErrorCode::InvalidCatalog => (
                "mod_market_catalog_invalid",
                "The Mod Market catalog is invalid.",
            ),
            ModMarketErrorCode::InvalidSignature => (
                "mod_market_signature_invalid",
                "The Mod Market catalog signature is invalid.",
            ),
            ModMarketErrorCode::InvalidPackage => (
                "mod_market_package_invalid",
                "The downloaded Mod package failed verification.",
            ),
            ModMarketErrorCode::DeploymentConflict => (
                "mod_proxy_deployment_conflict",
                "Existing game files differ from the managed components. No existing files were replaced.",
            ),
            ModMarketErrorCode::ItemNotFound => (
                "mod_market_item_not_found",
                "The selected Mod is no longer available in the market.",
            ),
        };
        Self {
            code,
            message_key,
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn mod_market_download_failed() -> Self {
        Self {
            code: "mod_market_download_failed",
            message_key: "Failed to connect to the Mod Market.",
            message_arguments: Vec::new(),
            diagnostic_line: None,
        }
    }

    pub(crate) fn mod_workspace_task_failed() -> Self {
        Self {
            code: "mod_workspace_task_failed",
            message_key: "The Mod workspace task did not complete.",
            message_arguments: vec![],
            diagnostic_line: None,
        }
    }
}
