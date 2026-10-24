use crate::process::git;

/// A repository as Trunk identifies it. On GitLab `owner` is the full namespace,
/// subgroups included (`group/sub`), and `name` is the project's last path segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl RepoRef {
    /// Split a `group/sub/project` path into owner and name.
    pub fn from_path(host: &str, path: &str) -> Option<Self> {
        let path = path.trim_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        let (owner, name) = path.rsplit_once('/')?;
        if owner.is_empty() || name.is_empty() {
            return None;
        }
        Some(RepoRef {
            host: host.to_string(),
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }

    pub fn path(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// Parse an https, ssh:// or scp-style (`git@host:group/project.git`) remote URL.
/// An https port is kept as part of the host; an ssh port is not, since it says
/// nothing about where the web UI and API live.
pub fn parse_remote_url(url: &str) -> Option<RepoRef> {
    let url = url.trim();
    let (host, path) = if let Some((scheme, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = if scheme.starts_with("http") {
            host
        } else {
            host.split(':').next()?
        };
        (host, path)
    } else {
        let (authority, path) = url.split_once(':')?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        (host, path)
    };
    RepoRef::from_path(host, path)
}

/// The repository behind `remote.origin.url`.
pub fn origin() -> Result<RepoRef, String> {
    let remote_url = git(&["config", "--get", "remote.origin.url"]);
    parse_remote_url(&remote_url)
        .ok_or_else(|| "Could not parse repository owner and name from remote URL".to_string())
}
