//! The endpoint table: what each provider calls its own API, and where.
//!
//! A `const` row per provider, because the set is closed and compiled in — `ProviderId` says so.
//! Which *flows* a provider offers is not here: [`ubiq_proto::connectors::ProviderId::flows`]
//! already answers that, and two tables saying the same thing is one table that goes stale.
//!
//! The client id is `option_env!`, read at **compile time only**. A build either ships a
//! registered application or it does not; reading the environment at run time would let a user's
//! shell decide which application a flow authenticates as, which is a different question from
//! which account it logs in.

use serde_json::Value;
use ubiq_proto::connectors::{ConnectError, ProviderId, origin};

/// One provider's endpoints, as this build knows them.
pub struct Provider {
    /// The API host for the provider's own cloud. Not always the web host — GitHub's API lives on
    /// `api.github.com` while its browser endpoints live on `github.com`.
    pub cloud_api: &'static str,
    /// The host a browser is sent to, and where the OAuth endpoints live.
    pub cloud_web: &'static str,
    /// What is appended to a self-hosted instance to reach its API. The cloud host already has it
    /// baked in, which is why the two are separate fields rather than one.
    pub api_base: &'static str,
    /// What a browser flow asks for, space separated as the specification wants it.
    pub scopes: &'static str,
    /// The endpoint that answers "who am I", under the API base.
    pub whoami: &'static str,
    /// Keys to look for in the whoami body, in order; the first present string wins. A body with
    /// none of them is not this product — which is how a Gitea URL typed into a GitLab connection
    /// fails at connect time rather than at first use.
    pub account_keys: &'static [&'static str],
    /// What the interface says above the paste field. Providers do not agree on a name for the
    /// thing, and the user is looking for the words their provider used.
    pub secret_prompt: &'static str,
    /// Browser endpoints, under the *web* root rather than the API base.
    pub authorize: &'static str,
    pub token: &'static str,
    /// Where a device flow starts. Empty for a provider with no device flow.
    pub device: &'static str,
    /// The application this build ships, if it ships one.
    pub client_id: Option<&'static str>,
}

const GITHUB: Provider = Provider {
    cloud_api: "https://api.github.com",
    cloud_web: "https://github.com",
    api_base: "/api/v3",
    scopes: "repo read:org read:user",
    whoami: "/user",
    account_keys: &["login"],
    secret_prompt: "Personal access token",
    authorize: "/login/oauth/authorize",
    token: "/login/oauth/access_token",
    device: "/login/device/code",
    client_id: option_env!("UBIQ_OAUTH_GITHUB_CLIENT_ID"),
};

const GITLAB: Provider = Provider {
    cloud_api: "https://gitlab.com",
    cloud_web: "https://gitlab.com",
    api_base: "/api/v4",
    scopes: "read_user api",
    whoami: "/user",
    account_keys: &["username"],
    secret_prompt: "Personal access token",
    authorize: "/oauth/authorize",
    token: "/oauth/token",
    device: "",
    client_id: option_env!("UBIQ_OAUTH_GITLAB_CLIENT_ID"),
};

const GITEA: Provider = Provider {
    // Gitea ships no cloud, so the two cloud fields are never reached — `instance_need` is
    // `Required` and a connection without an instance is refused before either is read.
    cloud_api: "",
    cloud_web: "",
    api_base: "/api/v1",
    scopes: "read:user repo",
    whoami: "/user",
    account_keys: &["login", "username"],
    secret_prompt: "Access token",
    authorize: "/login/oauth/authorize",
    token: "/login/oauth/access_token",
    device: "",
    client_id: option_env!("UBIQ_OAUTH_GITEA_CLIENT_ID"),
};

const AZURE_DEVOPS: Provider = Provider {
    cloud_api: "https://dev.azure.com",
    cloud_web: "https://app.vssps.visualstudio.com",
    api_base: "/_apis",
    scopes: "vso.profile vso.code",
    whoami: "/profile/profiles/me?api-version=7.0",
    account_keys: &["displayName", "emailAddress"],
    secret_prompt: "Personal access token",
    authorize: "/oauth2/authorize",
    token: "/oauth2/token",
    device: "",
    client_id: option_env!("UBIQ_OAUTH_AZURE_DEVOPS_CLIENT_ID"),
};

const ATLASSIAN: Provider = Provider {
    cloud_api: "https://api.atlassian.com",
    cloud_web: "https://auth.atlassian.com",
    api_base: "",
    scopes: "read:jira-user offline_access",
    whoami: "/rest/api/3/myself",
    account_keys: &["displayName", "emailAddress"],
    secret_prompt: "API token",
    authorize: "/authorize",
    token: "/oauth/token",
    device: "",
    client_id: option_env!("UBIQ_OAUTH_ATLASSIAN_CLIENT_ID"),
};

const GOOGLE: Provider = Provider {
    cloud_api: "https://www.googleapis.com",
    cloud_web: "https://accounts.google.com",
    api_base: "",
    scopes: "openid email profile",
    whoami: "/oauth2/v3/userinfo",
    account_keys: &["email"],
    secret_prompt: "Access token",
    authorize: "/o/oauth2/v2/auth",
    token: "/o/oauth2/token",
    device: "/o/oauth2/device/code",
    client_id: option_env!("UBIQ_OAUTH_GOOGLE_CLIENT_ID"),
};

/// The row for a provider.
pub fn of(provider: ProviderId) -> &'static Provider {
    match provider {
        ProviderId::Github => &GITHUB,
        ProviderId::Gitlab => &GITLAB,
        ProviderId::Gitea => &GITEA,
        ProviderId::AzureDevops => &AZURE_DEVOPS,
        ProviderId::Atlassian => &ATLASSIAN,
        ProviderId::Google => &GOOGLE,
    }
}

/// An API URL under `instance`, or under the provider's cloud when there is none.
///
/// The instance is *joined*, never rebuilt: an on-premises GitLab under `example.com/gitlab` keeps
/// its path, and a URL reconstructed from a host name would lose it. One trailing slash is trimmed
/// so `https://x/gitlab` and `https://x/gitlab/` are the same address.
///
/// Something that is not an absolute `http`/`https` URL is [`ConnectError::BadInstance`] here,
/// which is the earliest a bare host name typed into the instance field can be refused.
pub fn api_url(
    provider: ProviderId,
    instance: Option<&str>,
    path: &str,
) -> Result<String, ConnectError> {
    let row = of(provider);
    Ok(match instance {
        Some(instance) => format!("{}{}{path}", root(instance)?, row.api_base),
        None => format!("{}{path}", row.cloud_api),
    })
}

/// A URL at the instance root — the OAuth endpoints, which do not live under the API base.
pub fn web_url(
    provider: ProviderId,
    instance: Option<&str>,
    path: &str,
) -> Result<String, ConnectError> {
    Ok(match instance {
        Some(instance) => format!("{}{path}", root(instance)?),
        None => format!("{}{path}", of(provider).cloud_web),
    })
}

/// The origin a certificate is pinned against for this connection — the instance's, or the cloud
/// API host's.
pub fn instance_origin(
    provider: ProviderId,
    instance: Option<&str>,
) -> Result<String, ConnectError> {
    match instance {
        Some(instance) => origin(instance).ok_or(ConnectError::BadInstance),
        None => origin(of(provider).cloud_api).ok_or(ConnectError::BadInstance),
    }
}

/// The provider's own display name for the identity, out of a whoami body.
///
/// `None` means the body is not this product's: every row names at least one key, so a body that
/// has none of them answered something else.
pub fn account_name(provider: ProviderId, body: &Value) -> Option<String> {
    of(provider)
        .account_keys
        .iter()
        .find_map(|key| body.get(key)?.as_str())
        .map(str::to_string)
}

/// What the user typed, validated and with one trailing slash off.
fn root(instance: &str) -> Result<&str, ConnectError> {
    origin(instance).ok_or(ConnectError::BadInstance)?;
    Ok(instance.strip_suffix('/').unwrap_or(instance))
}
