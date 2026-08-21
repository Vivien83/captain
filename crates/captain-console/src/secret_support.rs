use captain_node::{NativeNodeProxySecrets, NodeProxyMode, ResolvedProxyPassword};
use std::path::Path;

pub(crate) fn resolve_proxy_password(
    proxy: &NodeProxyMode,
    _home: &Path,
) -> Result<Option<ResolvedProxyPassword>, ()> {
    NativeNodeProxySecrets::default()
        .resolve_proxy(proxy)
        .map_err(|_| ())
}
