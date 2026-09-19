use std::net::Ipv6Addr;

#[derive(Debug, Clone)]
pub(crate) struct CliEndpoint {
    base: reqwest::Url,
}

impl CliEndpoint {
    pub(crate) fn parse(value: &str) -> anyhow::Result<Self> {
        let value = value.trim();
        if value.is_empty() {
            anyhow::bail!("server endpoint cannot be empty");
        }

        let candidate = if value.contains("://") {
            value.to_string()
        } else if value.parse::<Ipv6Addr>().is_ok() {
            format!("http://[{value}]")
        } else {
            format!("http://{value}")
        };
        let mut base = reqwest::Url::parse(&candidate)
            .map_err(|_| anyhow::anyhow!("invalid server endpoint"))?;

        if !matches!(base.scheme(), "http" | "https") {
            anyhow::bail!("server endpoint must use http or https");
        }
        if base.host().is_none() {
            anyhow::bail!("server endpoint must include a host");
        }
        if base.query().is_some() || base.fragment().is_some() {
            anyhow::bail!("server endpoint must not include a query or fragment");
        }

        let base_path = base.path().trim_end_matches('/').to_string();
        base.set_path(&base_path);

        Ok(Self { base })
    }

    pub(crate) fn join(&self, path: &str) -> reqwest::Url {
        let path = path.trim_matches('/');
        if path.is_empty() {
            return self.base.clone();
        }

        let base_path = self.base.path().trim_end_matches('/');
        let joined_path = if base_path.is_empty() {
            format!("/{path}")
        } else {
            format!("{base_path}/{path}")
        };
        let mut url = self.base.clone();
        url.set_path(&joined_path);
        url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sql_url(endpoint: &str) -> String {
        CliEndpoint::parse(endpoint)
            .unwrap()
            .join("/sql")
            .to_string()
    }

    #[test]
    fn bare_endpoint_defaults_to_http() {
        assert_eq!(sql_url("localhost:3000"), "http://localhost:3000/sql");
        assert_eq!(sql_url("127.0.0.1:3000/"), "http://127.0.0.1:3000/sql");
    }

    #[test]
    fn explicit_http_and_https_schemes_are_preserved() {
        assert_eq!(
            sql_url("http://db.example.com:3000/"),
            "http://db.example.com:3000/sql"
        );
        assert_eq!(
            sql_url("https://db.example.com:3443///"),
            "https://db.example.com:3443/sql"
        );
    }

    #[test]
    fn base_paths_and_joined_paths_have_one_separator() {
        let endpoint = CliEndpoint::parse("https://db.example.com/api/v1///").unwrap();
        assert_eq!(
            endpoint.join("//sql//").as_str(),
            "https://db.example.com/api/v1/sql"
        );
    }

    #[test]
    fn ipv6_endpoints_are_supported() {
        assert_eq!(sql_url("[::1]:3000"), "http://[::1]:3000/sql");
        assert_eq!(sql_url("::1"), "http://[::1]/sql");
        assert_eq!(
            sql_url("https://[2001:db8::1]:3443/base/"),
            "https://[2001:db8::1]:3443/base/sql"
        );
    }

    #[test]
    fn invalid_endpoint_contract_is_rejected() {
        for endpoint in [
            "",
            "ftp://db.example.com",
            "https://db.example.com?mode=test",
            "https://db.example.com/#fragment",
        ] {
            assert!(CliEndpoint::parse(endpoint).is_err(), "{endpoint}");
        }
    }
}
