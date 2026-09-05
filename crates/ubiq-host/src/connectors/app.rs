//! Which OAuth application a flow authenticates as.
//!
//! Three sources, highest first: the connection's own client id, an application the user configured
//! for that provider and instance, and the one this build was compiled with. They are ordered by
//! how specific they are — a connection names one install, a settings row names one provider at one
//! origin, and the build's id is the fallback that only makes sense against a provider's cloud.
//!
//! A self-hosted install has no built-in answer at all, which is why
//! `ProviderId::needs_client_id` makes the interface ask before it opens anything.

use ubiq_proto::connectors::ProviderId;
use ubiq_proto::settings::HostSettings;

use super::providers;

/// Which of the three answered, so the interface can say so where the user is configuring it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientIdSource {
    /// The connection carries its own.
    Connection,
    /// An `OauthApp` row in settings matched this provider and origin.
    Settings,
    /// Compiled into this build.
    BuiltIn,
    /// Nothing answered: there is no browser flow to open.
    None,
}

impl ClientIdSource {
    /// The words the interface puts next to the field.
    pub fn label(self) -> &'static str {
        match self {
            ClientIdSource::Connection => "from this connection",
            ClientIdSource::Settings => "from settings",
            ClientIdSource::BuiltIn => "built in",
            ClientIdSource::None => "not configured",
        }
    }
}

/// The client id to authenticate as, and where it came from.
pub fn resolve(
    settings: &HostSettings,
    provider: ProviderId,
    origin: Option<&str>,
    own: Option<&str>,
) -> (Option<String>, ClientIdSource) {
    if let Some(own) = own.filter(|id| !id.is_empty()) {
        return (Some(own.to_string()), ClientIdSource::Connection);
    }
    let configured = settings
        .oauth_apps
        .iter()
        .find(|app| app.provider == provider && app.origin.as_deref() == origin)
        .map(|app| app.client_id.clone());
    if let Some(id) = configured {
        return (Some(id), ClientIdSource::Settings);
    }
    match providers::of(provider).client_id {
        Some(id) => (Some(id.to_string()), ClientIdSource::BuiltIn),
        None => (None, ClientIdSource::None),
    }
}

/// The id alone, for the flow that only needs to know whether there is one.
pub fn client_id(
    settings: &HostSettings,
    provider: ProviderId,
    origin: Option<&str>,
    own: Option<&str>,
) -> Option<String> {
    resolve(settings, provider, origin, own).0
}

/// Which source answered, for the interface that has to explain it.
pub fn client_id_source(
    settings: &HostSettings,
    provider: ProviderId,
    origin: Option<&str>,
    own: Option<&str>,
) -> ClientIdSource {
    resolve(settings, provider, origin, own).1
}
