
use resolve::resolver::DnsResolver;
use resolve::config::DnsConfig;

pub fn create_dht_router_addr(host: &str, port: usize, resolve: bool) -> Result<std::net::SocketAddr, String> {
    // resolve domain or just create router from IP
    if resolve {
        let dns_resolver_config = DnsConfig::with_name_servers(
            vec!["8.8.8.8:53".parse().unwrap(), "8.8.4.4:53".parse().unwrap()]
        );
        let dns_resolver = DnsResolver::new(dns_resolver_config).unwrap();
        match dns_resolver.resolve_host(host) {
            Ok(ip_addrs) => {
                // if there are multiple IPs just take one of them -> NO
                // TODO: we need all of them!!
                let ip: String = {
                    let mut rip = "".to_string();
                    for ip in ip_addrs {
                        rip = ip.to_string();
                        break;
                    }
                    rip
                };
                let ip_port = format!("{}:{}", ip, port);
                let dht_router = std::net::SocketAddr::V4(ip_port.parse().unwrap());
                return Ok(dht_router);
            },
            Err(e) => { return Err(format!("error_resolving_domain:{}", e).to_string()) },
        }
    } else {
        let ip_port = format!("{}:{}", host, port);
        let dht_router = std::net::SocketAddr::V4(ip_port.parse().unwrap());
        return Ok(dht_router);
    }
}
