//! iphlpapi.dll: the machine's adapters, as `net` gives its interfaces (`GetAdaptersAddresses`), and notifications of
//! address and route changes, which never come, as a machine's addresses never change while it runs; the rest is
//! declared so the programs that import it load, and says it isn't written yet.

use std::net::IpAddr;

use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "IPHLPAPI.dll";
    missing ConvertInterfaceGuidToLuid;
    host GetAdaptersAddresses(5) => get_adapters_addresses;
    missing GetAdaptersInfo;
    host GetBestInterface(2) => get_best_interface;
    missing GetExtendedTcpTable;
    missing GetIfEntry2;
    host NotifyAddrChange(2) => notify_change;
    // Not written yet: what larger programs import.
    host CancelIPChangeNotify(1) => cancel_ip_change_notify;
    missing FreeMibTable;
    missing GetIfTable2;
    missing GetInterfaceInfo;
    missing IpReleaseAddress;
    missing IpRenewAddress;
    host NotifyRouteChange(2) => notify_change;
}

/// ERROR_IO_PENDING, and ERROR_NOT_SUPPORTED.
const ERROR_IO_PENDING: u64 = 997;
const ERROR_NOT_SUPPORTED: u64 = 50;

/// `NotifyAddrChange(&handle, &overlapped)` and `NotifyRouteChange`: pending, its event reset, for a change that never
/// comes (until `CancelIPChangeNotify`). Waiting for one without an OVERLAPPED would wait for ever, so isn't done.
fn notify_change(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] == 0 {
        return Reply::Value(ERROR_NOT_SUPPORTED);
    }
    let abi = task.abi();
    let event = task.mem(cx, false, |t| user::read_ptr(&t.space(), a[1] + abi.overlapped_event(), abi.width)).ok().filter(|&e| e != 0).and_then(|e| task.object(e));
    if let Some(event) = &event {
        task.k.objects.signal(event, |s| s.set = false);
    }
    let pending = abi.overlapped_result(crate::ports::STATUS_PENDING, 0);
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &pending));
    if a[0] != 0 {
        let _ = task.write_ptr(cx, a[0], 0);
    }
    Reply::Value(ERROR_IO_PENDING)
}

/// `CancelIPChangeNotify(&overlapped)`: there's nothing under way to finish, as nothing ever completes it.
fn cancel_ip_change_notify(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1)
}

// ---- GetAdaptersAddresses ----

/// ERROR_BUFFER_OVERFLOW, ERROR_INVALID_PARAMETER and ERROR_NO_DATA.
const ERROR_BUFFER_OVERFLOW: u64 = 111;
const ERROR_INVALID_PARAMETER: u64 = 87;
const ERROR_NO_DATA: u64 = 232;

/// GAA_FLAG_*: what to leave out, or put in.
const SKIP_UNICAST: u32 = 0x1;
const SKIP_DNS_SERVER: u32 = 0x8;
const INCLUDE_PREFIX: u32 = 0x10;
const SKIP_FRIENDLY_NAME: u32 = 0x20;
const INCLUDE_GATEWAYS: u32 = 0x80;

/// AF_INET and AF_INET6.
const AF_INET: u32 = 2;
const AF_INET6: u32 = 23;

/// IF_TYPE_ETHERNET_CSMACD and IF_TYPE_SOFTWARE_LOOPBACK.
const IF_ETHERNET: u32 = 6;
const IF_LOOPBACK: u32 = 24;

/// A buffer of structures as the program's ISA lays them out: fields at their alignment (8-byte ones on 8, on 32-bit
/// x86 too, as MSVC does), pointers its width. A pointer is to an offset in the buffer, filled in once its address is
/// known.
struct Layout {
    bytes: Vec<u8>,
    width: usize,
    /// Where each pointer is, and the offset it points to.
    pointers: Vec<(usize, usize)>,
}

impl Layout {
    fn new(width: u64) -> Layout {
        Layout { bytes: Vec::new(), width: width as usize, pointers: Vec::new() }
    }
    fn align(&mut self, n: usize) {
        while !self.bytes.len().is_multiple_of(n) {
            self.bytes.push(0);
        }
    }
    fn at(&self) -> usize {
        self.bytes.len()
    }
    fn u8s(&mut self, v: &[u8]) {
        self.bytes.extend(v);
    }
    fn u32(&mut self, v: u32) {
        self.align(4);
        self.bytes.extend(v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.align(8);
        self.bytes.extend(v.to_le_bytes());
    }
    /// A pointer field, null for now: its place, for `point`.
    fn ptr(&mut self) -> usize {
        self.align(self.width);
        let at = self.at();
        self.bytes.extend(std::iter::repeat_n(0, self.width));
        at
    }
    /// The pointer at `field` points at `to`.
    fn point(&mut self, field: usize, to: usize) {
        self.pointers.push((field, to));
    }
    /// A `SOCKET_ADDRESS`: the sockaddr's pointer and its length; the pointer's place.
    fn socket_address(&mut self, len: usize) -> usize {
        let at = self.ptr();
        self.u32(len as u32);
        self.align(self.width);
        at
    }
    /// The structure that started at `start` ends here, on 8 bytes: its length.
    fn end(&mut self, start: usize) -> usize {
        self.align(8);
        self.at() - start
    }
    /// Data the pointer at `field` points at, on `align`.
    fn data(&mut self, field: usize, align: usize, bytes: &[u8]) {
        self.align(align);
        let at = self.at();
        self.bytes.extend(bytes);
        self.point(field, at);
    }
    fn wide(&mut self, field: usize, s: &str) {
        let units: Vec<u8> = s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
        self.data(field, 2, &units);
    }
    /// The bytes, as they'll be at `base`.
    fn at_base(mut self, base: u64) -> Vec<u8> {
        for (field, to) in std::mem::take(&mut self.pointers) {
            let v = (base + to as u64).to_le_bytes();
            self.bytes[field..field + self.width].copy_from_slice(&v[..self.width]);
        }
        self.bytes
    }
}

/// A list of fixed-size records (each `Length`, a word to spare, `Next`, then a `SOCKET_ADDRESS`, then `extra`),
/// one per sockaddr, chained from the pointer at `first`. Prefix records carry their length in `extra`.
fn chain(l: &mut Layout, first: usize, items: &[(Vec<u8>, Option<u32>)]) {
    let mut link = first;
    for (sockaddr, prefix) in items {
        l.align(8);
        let start = l.at();
        l.point(link, start);
        let length = l.at();
        l.u32(0);
        l.u32(0);
        link = l.ptr();
        let address = l.socket_address(sockaddr.len());
        if let Some(p) = prefix {
            l.u32(*p);
        }
        let len = l.end(start) as u32;
        l.bytes[length..length + 4].copy_from_slice(&len.to_le_bytes());
        l.data(address, 4, sockaddr);
    }
}

/// An address's sockaddr, with no port.
fn sockaddr(ip: IpAddr) -> Vec<u8> {
    crate::sockets::sockaddr(&std::net::SocketAddr::new(ip, 0))
}

/// The first address of `ip`'s network, `prefix` bits long.
fn network(ip: IpAddr, prefix: u8) -> IpAddr {
    match ip {
        IpAddr::V4(v4) => {
            let mask = if prefix == 0 { 0 } else { u32::MAX << (32 - u32::from(prefix)) };
            IpAddr::V4((u32::from(v4) & mask).into())
        }
        IpAddr::V6(v6) => {
            let mask = if prefix == 0 { 0 } else { u128::MAX << (128 - u32::from(prefix)) };
            IpAddr::V6((u128::from(v6) & mask).into())
        }
    }
}

/// Every adapter's `IP_ADAPTER_ADDRESSES_LH`, chained, with what they point at after each: the ethernet adapter
/// first and the loopback after it, as Windows lists them.
fn adapters(net: &net::Network, width: u64, family: u32, flags: u32) -> Layout {
    let mut interfaces = net.interfaces();
    interfaces.sort_by_key(|i| i.loopback);
    let wanted = |ip: &IpAddr| match family {
        AF_INET => ip.is_ipv4(),
        AF_INET6 => ip.is_ipv6(),
        _ => true,
    };
    let mut l = Layout::new(width);
    let mut next: Option<usize> = None;
    for i in &interfaces {
        let addrs: Vec<(IpAddr, u8)> = i.addrs.iter().copied().filter(|(ip, _)| wanted(ip)).collect();
        let (v4, v6) = (i.addrs.iter().any(|(ip, _)| ip.is_ipv4()), i.addrs.iter().any(|(ip, _)| ip.is_ipv6()));
        let if_type = if i.loopback { IF_LOOPBACK } else { IF_ETHERNET };
        l.align(8);
        let start = l.at();
        if let Some(field) = next {
            l.point(field, start);
        }
        let length = l.at();
        l.u32(0);
        l.u32(i.index);
        next = Some(l.ptr());
        let adapter_name = l.ptr();
        let first_unicast = l.ptr();
        let _anycast = l.ptr();
        let _multicast = l.ptr();
        let first_dns = l.ptr();
        let dns_suffix = l.ptr();
        let description = l.ptr();
        let friendly_name = l.ptr();
        let mut mac = [0u8; 8];
        let mac_len = if i.loopback { 0 } else { 6 };
        mac[..6].copy_from_slice(&i.mac);
        l.u8s(&mac);
        l.u32(mac_len);
        // IP_ADAPTER_DDNS_ENABLED, IP_ADAPTER_DHCP_ENABLED (ethernet), IPV4_ENABLED and IPV6_ENABLED.
        l.u32(0x1 | if i.loopback { 0 } else { 0x4 } | if v4 { 0x80 } else { 0 } | if v6 { 0x100 } else { 0 });
        l.u32(i.mtu);
        l.u32(if_type);
        l.u32(1); // IfOperStatusUp
        l.u32(if v6 { i.index } else { 0 });
        for _ in 0..16 {
            l.u32(0);
        }
        let first_prefix = l.ptr();
        let speed = if i.loopback { 1_073_741_824_000 } else { 1_000_000_000 };
        l.u64(speed);
        l.u64(speed);
        let _wins = l.ptr();
        let first_gateway = l.ptr();
        l.u32(if i.loopback { 75 } else { 25 });
        l.u32(if i.loopback { 75 } else { 25 });
        // NET_LUID: the interface's index from 24, its type from 48.
        l.u64(u64::from(i.index) << 24 | u64::from(if_type) << 48);
        let _dhcp4 = l.socket_address(0);
        l.u32(1); // CompartmentId
        l.u8s(&guid(i.index, 0x4e));
        l.u32(1); // NET_IF_CONNECTION_DEDICATED
        l.u32(0); // TUNNEL_TYPE_NONE
        let _dhcp6 = l.socket_address(0);
        l.u8s(&[0; 130]);
        l.u32(0);
        l.u32(0);
        let _dns_suffixes = l.ptr();
        let len = l.end(start) as u32;
        l.bytes[length..length + 4].copy_from_slice(&len.to_le_bytes());

        // What it points at.
        let name = format!("{{{}}}", guid_text(i.index, 0x41));
        l.data(adapter_name, 1, format!("{name}\0").as_bytes());
        if flags & SKIP_UNICAST == 0 {
            let mut link = first_unicast;
            for &(ip, prefix) in &addrs {
                l.align(8);
                let start = l.at();
                l.point(link, start);
                let length = l.at();
                l.u32(0);
                l.u32(if i.loopback { 0 } else { 1 }); // IP_ADAPTER_ADDRESS_DNS_ELIGIBLE
                link = l.ptr();
                let address = l.socket_address(sockaddr(ip).len());
                let origin = if i.loopback { 2 } else if ip.is_ipv4() { 3 } else { 2 }; // well known, or DHCP
                l.u32(origin);
                l.u32(origin);
                l.u32(4); // IpDadStatePreferred
                l.u32(u32::MAX);
                l.u32(u32::MAX);
                l.u32(u32::MAX);
                l.u8s(&[prefix]);
                let len = l.end(start) as u32;
                l.bytes[length..length + 4].copy_from_slice(&len.to_le_bytes());
                l.data(address, 4, &sockaddr(ip));
            }
        }
        let gateway = IpAddr::V4(net.gateway());
        if flags & SKIP_DNS_SERVER == 0 && !i.loopback && wanted(&gateway) {
            chain(&mut l, first_dns, &[(sockaddr(gateway), None)]);
        }
        if flags & INCLUDE_PREFIX != 0 {
            let prefixes: Vec<_> = addrs.iter().map(|&(ip, p)| (sockaddr(network(ip, p)), Some(u32::from(p)))).collect();
            chain(&mut l, first_prefix, &prefixes);
        }
        if flags & INCLUDE_GATEWAYS != 0 && !i.loopback && wanted(&gateway) {
            chain(&mut l, first_gateway, &[(sockaddr(gateway), None)]);
        }
        l.wide(dns_suffix, "");
        let (friendly, describe) = if i.loopback { ("Loopback Pseudo-Interface 1", "Software Loopback Interface 1") } else { ("Ethernet", "Carbon Network Adapter") };
        l.wide(description, describe);
        l.wide(friendly_name, if flags & SKIP_FRIENDLY_NAME != 0 { "" } else { friendly });
    }
    l
}

/// A GUID for interface `index`, the same every run, `kind` telling the adapter's name from its network's.
fn guid(index: u32, kind: u8) -> [u8; 16] {
    let mut g = [0u8; 16];
    g[..4].copy_from_slice(&0xc0a7_0000u32.wrapping_add(index).to_le_bytes());
    g[4..6].copy_from_slice(&u16::from(kind).to_le_bytes());
    g[6..8].copy_from_slice(&0x4000u16.to_le_bytes());
    g[8] = 0x80;
    g[15] = index as u8;
    g
}

fn guid_text(index: u32, kind: u8) -> String {
    let g = guid(index, kind);
    let d1 = u32::from_le_bytes(g[..4].try_into().unwrap());
    let d2 = u16::from_le_bytes([g[4], g[5]]);
    let d3 = u16::from_le_bytes([g[6], g[7]]);
    let tail: String = g[10..].iter().map(|b| format!("{b:02X}")).collect();
    format!("{d1:08X}-{d2:04X}-{d3:04X}-{:02X}{:02X}-{tail}", g[8], g[9])
}

/// `GetAdaptersAddresses(family, flags, reserved, buffer, &size)`: the adapters, or ERROR_BUFFER_OVERFLOW and the
/// size they need.
fn get_adapters_addresses(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (family, flags, buffer, size_at) = (a[0] as u32, a[1] as u32, a[3], a[4]);
    if size_at == 0 || !matches!(family, 0 | AF_INET | AF_INET6) {
        return Reply::Value(ERROR_INVALID_PARAMETER);
    }
    let Ok(room) = task.mem(cx, false, |t| user::read_u32(&t.space(), size_at)) else { return Reply::Value(ERROR_INVALID_PARAMETER) };
    let layout = adapters(&task.k.net, task.abi().width, family, flags);
    let need = layout.bytes.len();
    if need == 0 {
        return Reply::Value(ERROR_NO_DATA);
    }
    if buffer == 0 || (room as usize) < need {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), size_at, need as u32));
        return Reply::Value(ERROR_BUFFER_OVERFLOW);
    }
    let bytes = layout.at_base(buffer);
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), buffer, &bytes));
    let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), size_at, need as u32));
    Reply::Value(0)
}

/// `GetBestInterface(address, &index)`: the interface a packet to the IPv4 address (in network order) would leave by:
/// the loopback for 127.0.0.0/8, else the machine's ethernet adapter, whose route is the default one.
fn get_best_interface(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] == 0 {
        return Reply::Value(ERROR_INVALID_PARAMETER);
    }
    let address = std::net::Ipv4Addr::from((a[0] as u32).to_le_bytes());
    let interfaces = task.k.net.interfaces();
    let index = interfaces.iter().find(|i| i.loopback == address.is_loopback()).map_or(0, |i| i.index);
    if index == 0 {
        return Reply::Value(ERROR_NO_DATA);
    }
    if task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[1], index)).is_err() {
        return Reply::Value(ERROR_INVALID_PARAMETER);
    }
    Reply::Value(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The records' sizes, as Windows' headers give them.
    #[test]
    fn records_are_windows_sizes() {
        let net = net::Network::on_this_host(net::Config { hostname: "m".into(), dns: None });
        for (width, adapter, unicast, first_unicast) in [(8u64, 448u32, 64u32, 24usize), (4, 376, 48, 16)] {
            let l = adapters(&net, width, 0, 0);
            assert_eq!(u32::from_le_bytes(l.bytes[0..4].try_into().unwrap()), adapter);
            let bytes = l.at_base(0x1000);
            let w = width as usize;
            let ptr = |at: usize| {
                let mut b = [0u8; 8];
                b[..w].copy_from_slice(&bytes[at..at + w]);
                u64::from_le_bytes(b)
            };
            // The ethernet adapter's first unicast address, its length, and its IPv4 sockaddr.
            let u = (ptr(first_unicast) - 0x1000) as usize;
            assert_eq!(u32::from_le_bytes(bytes[u..u + 4].try_into().unwrap()), unicast);
            let sa = (ptr(u + 8 + w) - 0x1000) as usize;
            assert_eq!(u16::from_le_bytes([bytes[sa], bytes[sa + 1]]), 2);
            // Next leads to the loopback adapter, index 1, whose own Next ends the list.
            let second = (ptr(8) - 0x1000) as usize;
            assert_eq!(u32::from_le_bytes(bytes[second + 4..second + 8].try_into().unwrap()), 1);
            assert_eq!(ptr(second + 8), 0);
        }
    }

    #[test]
    fn best_interfaces() {
        let net = net::Network::on_this_host(net::Config { hostname: "m".into(), dns: None });
        let loopback = std::net::Ipv4Addr::from(u32::from_le_bytes([127, 0, 0, 1]).to_le_bytes());
        assert!(loopback.is_loopback());
        assert!(net.interfaces().iter().any(|i| i.loopback) && net.interfaces().iter().any(|i| !i.loopback));
    }

    #[test]
    fn networks_and_guids() {
        assert_eq!(network("10.1.2.3".parse().unwrap(), 16), "10.1.0.0".parse::<IpAddr>().unwrap());
        assert_eq!(network("::1".parse().unwrap(), 128), "::1".parse::<IpAddr>().unwrap());
        assert_eq!(guid_text(2, 0x41).len(), 36);
    }
}
