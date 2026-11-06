//! Follow configured proxies for remote services; keep device-local services local.
pub(crate) fn client_builder(base: &str) -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();
    let local = url::Url::parse(base)
        .ok()
        .is_some_and(|url| match url.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            Some(url::Host::Domain(name)) => matches!(name, "localhost" | "localhost."),
            None => false,
        });
    if local { builder.no_proxy() } else { builder }
}
