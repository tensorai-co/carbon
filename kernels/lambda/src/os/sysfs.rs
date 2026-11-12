//! sysfs: `/sys`, made each time it's read, as `/proc` is. As much of it as programs and libraries read: the CPUs
//! (online, possible, present, each one's topology), memory's NUMA node, the network interfaces (each one's address,
//! MTU, state, index and type, with `/sys/class/net`'s links to them), the character devices lambda has (each one's
//! `dev` and `uevent`, with `/sys/dev/char`'s links by number, which libudev goes through from a device file), the
//! classes a device scan looks in (`drm`, `input`, `block`, all empty until there are such devices), and transparent
//! huge pages' setting. Its links point within it, and are followed here.

use crate::Kernel;
use crate::abi::*;

/// What's at a sysfs path.
pub(crate) enum Entry {
    /// A directory, and what's in it (each name and its `DT_*` type).
    Dir(Vec<(String, u8)>),
    File(String),
    /// A link, and its target (relative to its directory, as sysfs's are).
    Link(String),
}

/// The character devices lambda has: each one's name, class, number, and the directory of devices it's in.
const DEVICES: [(&str, &str, u32, u32); 7] = [
    ("null", "mem", 1, 3),
    ("zero", "mem", 1, 5),
    ("random", "mem", 1, 8),
    ("urandom", "mem", 1, 9),
    ("tty", "tty", 5, 0),
    ("console", "tty", 5, 1),
    ("ptmx", "tty", 5, 2),
];

fn dir(names: &[&str]) -> Entry {
    Entry::Dir(names.iter().map(|n| (n.to_string(), DT_DIR)).collect())
}

fn files(names: &[&str]) -> Vec<(String, u8)> {
    names.iter().map(|n| (n.to_string(), DT_REG)).collect()
}

/// A CPU list as sysfs writes one: `0-3`, or `0` for one.
fn cpu_list(n: usize) -> String {
    if n <= 1 { "0\n".to_string() } else { format!("0-{}\n", n - 1) }
}

/// A set of CPUs as sysfs writes a mask: 32-bit hexadecimal groups, the highest first, with commas between.
fn cpu_mask(cpus: impl Iterator<Item = usize>, count: usize) -> String {
    let mut words = vec![0u32; count.max(1).div_ceil(32)];
    for c in cpus {
        words[c / 32] |= 1 << (c % 32);
    }
    let mut out: Vec<String> = words.iter().rev().map(|w| format!("{w:08x}")).collect();
    out[0] = format!("{:x}", words[words.len() - 1]);
    out.join(",") + "\n"
}

/// What's at a path with no links in it (a path from `/sys`, no trailing slash).
fn entry(k: &Kernel, path: &str) -> Option<Entry> {
    let cpus = k.host.cpus().max(1);
    let interfaces = k.net.interfaces();
    let parts: Vec<&str> = path.trim_start_matches("/sys").split('/').filter(|p| !p.is_empty()).collect();
    Some(match parts.as_slice() {
        [] => dir(&["block", "bus", "class", "dev", "devices", "firmware", "fs", "kernel", "module", "power"]),
        ["block"] | ["firmware"] | ["module"] | ["power"] | ["fs", "cgroup"] | ["kernel", "security"] => Entry::Dir(Vec::new()),
        ["bus"] => dir(&["pci"]),
        ["bus", "pci"] => dir(&["devices", "drivers"]),
        ["bus", "pci", "devices" | "drivers"] => Entry::Dir(Vec::new()),
        ["fs"] => dir(&["cgroup"]),
        ["kernel"] => dir(&["mm", "security"]),
        ["kernel", "mm"] => dir(&["transparent_hugepage"]),
        ["kernel", "mm", "transparent_hugepage"] => Entry::Dir(files(&["enabled", "defrag"])),
        ["kernel", "mm", "transparent_hugepage", "enabled"] => Entry::File("always madvise [never]\n".into()),
        ["kernel", "mm", "transparent_hugepage", "defrag"] => Entry::File("always defer defer+madvise madvise [never]\n".into()),
        ["class"] => dir(&["block", "drm", "input", "mem", "net", "tty"]),
        ["class", "block" | "drm" | "input"] => Entry::Dir(Vec::new()),
        ["class", "net"] => Entry::Dir(interfaces.iter().map(|i| (i.name.to_string(), DT_LNK)).collect()),
        ["class", "net", name] => {
            interfaces.iter().find(|i| i.name == *name)?;
            Entry::Link(format!("../../devices/virtual/net/{name}"))
        }
        ["class", class @ ("mem" | "tty")] => Entry::Dir(DEVICES.iter().filter(|d| d.1 == *class).map(|d| (d.0.to_string(), DT_LNK)).collect()),
        ["class", class @ ("mem" | "tty"), name] => {
            DEVICES.iter().find(|d| d.1 == *class && d.0 == *name)?;
            Entry::Link(format!("../../devices/virtual/{class}/{name}"))
        }
        ["dev"] => dir(&["block", "char"]),
        ["dev", "block"] => Entry::Dir(Vec::new()),
        ["dev", "char"] => Entry::Dir(DEVICES.iter().map(|d| (format!("{}:{}", d.2, d.3), DT_LNK)).collect()),
        ["dev", "char", number] => {
            let d = DEVICES.iter().find(|d| format!("{}:{}", d.2, d.3) == *number)?;
            Entry::Link(format!("../../devices/virtual/{}/{}", d.1, d.0))
        }
        ["devices"] => dir(&["system", "virtual"]),
        ["devices", "virtual"] => dir(&["mem", "net", "tty"]),
        ["devices", "virtual", "net"] => Entry::Dir(interfaces.iter().map(|i| (i.name.to_string(), DT_DIR)).collect()),
        ["devices", "virtual", "net", name] => {
            interfaces.iter().find(|i| i.name == *name)?;
            Entry::Dir(files(&["address", "addr_len", "broadcast", "carrier", "flags", "ifindex", "iflink", "mtu", "operstate", "type", "uevent"]))
        }
        ["devices", "virtual", "net", name, attribute] => {
            let i = interfaces.iter().find(|i| i.name == *name)?;
            let mac = |m: [u8; 6]| m.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":");
            Entry::File(match *attribute {
                "address" => format!("{}\n", mac(i.mac)),
                "addr_len" => "6\n".into(),
                "broadcast" => format!("{}\n", mac(if i.loopback { [0; 6] } else { [0xff; 6] })),
                "carrier" => "1\n".into(),
                // Up and running; a loopback's, and broadcast and multicast for another.
                "flags" => format!("{:#x}\n", if i.loopback { 0x9 } else { 0x1003 }),
                "ifindex" | "iflink" => format!("{}\n", i.index),
                "mtu" => format!("{}\n", i.mtu),
                "operstate" => if i.loopback { "unknown\n".into() } else { "up\n".into() },
                "type" => if i.loopback { "772\n".into() } else { "1\n".into() },
                "uevent" => format!("INTERFACE={}\nIFINDEX={}\n", i.name, i.index),
                _ => return None,
            })
        }
        ["devices", "virtual", class @ ("mem" | "tty")] => Entry::Dir(DEVICES.iter().filter(|d| d.1 == *class).map(|d| (d.0.to_string(), DT_DIR)).collect()),
        ["devices", "virtual", class @ ("mem" | "tty"), name] => {
            DEVICES.iter().find(|d| d.1 == *class && d.0 == *name)?;
            Entry::Dir(files(&["dev", "uevent"]))
        }
        ["devices", "virtual", class @ ("mem" | "tty"), name, attribute] => {
            let d = DEVICES.iter().find(|d| d.1 == *class && d.0 == *name)?;
            Entry::File(match *attribute {
                "dev" => format!("{}:{}\n", d.2, d.3),
                "uevent" => format!("MAJOR={}\nMINOR={}\nDEVNAME={}\nDEVMODE=0666\n", d.2, d.3, d.0),
                _ => return None,
            })
        }
        ["devices", "system"] => dir(&["cpu", "node"]),
        ["devices", "system", "cpu"] => {
            let mut list: Vec<(String, u8)> = (0..cpus).map(|i| (format!("cpu{i}"), DT_DIR)).collect();
            list.extend(files(&["online", "possible", "present", "offline", "kernel_max"]));
            Entry::Dir(list)
        }
        ["devices", "system", "cpu", "online" | "possible" | "present"] => Entry::File(cpu_list(cpus)),
        ["devices", "system", "cpu", "offline"] => Entry::File("\n".into()),
        ["devices", "system", "cpu", "kernel_max"] => Entry::File("8191\n".into()),
        ["devices", "system", "cpu", cpu, rest @ ..] if cpu.strip_prefix("cpu").and_then(|n| n.parse::<usize>().ok()).is_some_and(|n| n < cpus) => {
            let n: usize = cpu[3..].parse().ok()?;
            match rest {
                // cpu0 can't be taken offline, so it has no `online`.
                [] => Entry::Dir([files(if n == 0 { &[] } else { &["online"] }), vec![("topology".to_string(), DT_DIR)]].concat()),
                ["online"] if n > 0 => Entry::File("1\n".into()),
                ["topology"] => Entry::Dir(files(&[
                    "core_id", "physical_package_id", "cluster_id", "die_id", "core_cpus", "core_cpus_list", "thread_siblings", "thread_siblings_list",
                    "core_siblings", "core_siblings_list", "cluster_cpus", "cluster_cpus_list", "package_cpus", "package_cpus_list",
                ])),
                ["topology", "core_id"] => Entry::File(format!("{n}\n")),
                ["topology", "physical_package_id" | "cluster_id" | "die_id"] => Entry::File("0\n".into()),
                // One thread a core, every core in one package.
                ["topology", "core_cpus_list" | "thread_siblings_list"] => Entry::File(format!("{n}\n")),
                ["topology", "core_cpus" | "thread_siblings"] => Entry::File(cpu_mask(std::iter::once(n), cpus)),
                ["topology", "core_siblings_list" | "cluster_cpus_list" | "package_cpus_list"] => Entry::File(cpu_list(cpus)),
                ["topology", "core_siblings" | "cluster_cpus" | "package_cpus"] => Entry::File(cpu_mask(0..cpus, cpus)),
                _ => return None,
            }
        }
        ["devices", "system", "node"] => Entry::Dir([vec![("node0".to_string(), DT_DIR)], files(&["online", "possible", "has_cpu", "has_memory"])].concat()),
        ["devices", "system", "node", "online" | "possible" | "has_cpu" | "has_memory"] => Entry::File("0\n".into()),
        ["devices", "system", "node", "node0"] => Entry::Dir(files(&["cpulist", "cpumap"])),
        ["devices", "system", "node", "node0", "cpulist"] => Entry::File(cpu_list(cpus)),
        ["devices", "system", "node", "node0", "cpumap"] => Entry::File(cpu_mask(0..cpus, cpus)),
        _ => return None,
    })
}

/// Whether a path is in `/sys`.
pub(crate) fn is_sys(path: &str) -> bool {
    path == "/sys" || path.starts_with("/sys/")
}

/// What a path names, its links followed (the last one too, if `follow`): the path with no links in it, and its
/// entry. None if it isn't in `/sys` or names nothing there.
pub(crate) fn lookup(k: &Kernel, path: &str, follow: bool) -> Option<(String, Entry)> {
    if !is_sys(path) {
        return None;
    }
    let mut rest: std::collections::VecDeque<String> = path.split('/').filter(|p| !p.is_empty() && *p != ".").map(String::from).collect();
    let mut walked: Vec<String> = Vec::new();
    let mut links = 0;
    while let Some(part) = rest.pop_front() {
        if part == ".." {
            walked.pop();
            continue;
        }
        walked.push(part);
        let here = format!("/{}", walked.join("/"));
        if !is_sys(&here) {
            return None;
        }
        match entry(k, &here)? {
            Entry::Link(target) if follow || !rest.is_empty() => {
                links += 1;
                if links > 40 {
                    return None;
                }
                walked.pop();
                for p in target.split('/').rev().filter(|p| !p.is_empty()) {
                    rest.push_front(p.to_string());
                }
            }
            e if rest.is_empty() => return Some((here, e)),
            Entry::Dir(_) | Entry::Link(_) => {}
            Entry::File(_) => return None,
        }
    }
    let here = format!("/{}", walked.join("/"));
    entry(k, &here).map(|e| (here, e))
}

#[cfg(test)]
mod tests {
    use super::{cpu_list, cpu_mask};

    #[test]
    fn cpu_lists_and_masks() {
        assert_eq!(cpu_list(1), "0\n");
        assert_eq!(cpu_list(8), "0-7\n");
        assert_eq!(cpu_mask(0..8, 8), "ff\n");
        assert_eq!(cpu_mask(std::iter::once(3), 8), "8\n");
        assert_eq!(cpu_mask(0..40, 40), "ff,ffffffff\n");
    }
}
