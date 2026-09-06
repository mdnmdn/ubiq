//! Which OAuth application a flow authenticates as.
//!
//! Four sources, highest first: the connection's own client id, the registration a connection or a
//! flow names, a registration matching the provider and instance, and the one this build was
//! compiled with. They are ordered by how specific they are — a connection names one application,
//! a named registration is the user's own choice for this flow, a matched registration is the only
//! one on that install, and the build's id is the fallback that only makes sense against a
//! provider's cloud.
//!
//! A self-hosted install has no built-in answer at all, which is why
//! `ProviderId::needs_client_id` makes the interface ask before it opens anything.

use ubiq_proto::connectors::ProviderId;
use ubiq_proto::ids::OauthAppId;
use ubiq_proto::settings::HostSettings;

use super::providers;

/// Which of the sources answered, so the interface can say so where the user is configuring it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientIdSource {
    /// The connection carries its own.
    Connection,
    /// A registration answered. It carries which one, because a provider and an instance no longer
    /// name a single registration and "from settings" would leave the user guessing between them.
    Registration(OauthAppId),
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
            ClientIdSource::Registration(_) => "from an app registration",
            ClientIdSource::BuiltIn => "built in",
            ClientIdSource::None => "not configured",
        }
    }

    /// The registration that answered, where one did. What a stored client secret is filed under.
    pub fn registration(self) -> Option<OauthAppId> {
        match self {
            ClientIdSource::Registration(app) => Some(app),
            _ => None,
        }
    }
}

/// The client id to authenticate as, and where it came from.
pub fn resolve(
    settings: &HostSettings,
    provider: ProviderId,
    origin: Option<&str>,
    own: Option<&str>,
    app: Option<OauthAppId>,
) -> (Option<String>, ClientIdSource) {
    if let Some(own) = own.filter(|id| !id.is_empty()) {
        return (Some(own.to_string()), ClientIdSource::Connection);
    }
    // A named registration wins over one merely matching the instance: the user picked it, and a
    // probe months later must authenticate as whatever the connection was made under.
    let named = app.and_then(|app| settings.oauth_apps.iter().find(|held| held.id == app));
    let matched = || {
        settings
            .oauth_apps
            .iter()
            .find(|held| held.provider == provider && held.origin.as_deref() == origin)
    };
    if let Some(held) = named
        .or_else(matched)
        .filter(|held| !held.client_id.is_empty())
    {
        return (
            Some(held.client_id.clone()),
            ClientIdSource::Registration(held.id),
        );
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
    app: Option<OauthAppId>,
) -> Option<String> {
    resolve(settings, provider, origin, own, app).0
}

/// Which source answered, for the interface that has to explain it.
pub fn client_id_source(
    settings: &HostSettings,
    provider: ProviderId,
    origin: Option<&str>,
    own: Option<&str>,
    app: Option<OauthAppId>,
) -> ClientIdSource {
    resolve(settings, provider, origin, own, app).1
}
