
use resolve::resolver::DnsResolver;
use resolve::config::DnsConfig;

use crate::bittorrent::dht::DHTServer;

pub fn create_dht_router_addr(server: DHTServer) -> Result<Vec<std::net::SocketAddr>, String> {
    // resolve domain or just create router from IP
    match server {
        DHTServer::DomainNameServer(host, port) => {
            let dns_resolver_config = DnsConfig::with_name_servers(
                vec!["8.8.8.8:53".parse().unwrap(), "8.8.4.4:53".parse().unwrap()]
            );
            let dns_resolver = DnsResolver::new(dns_resolver_config).unwrap();
            match dns_resolver.resolve_host(&host) {
                Ok(ip_addrs) => {
                    let mut result = Vec::with_capacity(10);
                    for ip in ip_addrs {
                        let dht_router = std::net::SocketAddr::new(ip, port);
                        result.push(dht_router);
                    }
                    return Ok(result);
                },
                Err(e) => { return Err(format!("error_resolving_domain:{}", e).to_string()) },
            }
        },
        DHTServer::IPServer(host, port) => {
            let ip_port = format!("{}:{}", host, port);
            let dht_router = std::net::SocketAddr::V4(ip_port.parse().unwrap());
            return Ok(vec![dht_router]);
        },
    }
}
