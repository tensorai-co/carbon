//! Who a machine's programs run as: `--user`, an `exec`'s `user`, or the image's `USER`, read as Docker reads it:
//! `name` or `uid`, then `:group` or `:gid`, from the machine's own `/etc/passwd` and `/etc/group`. A user with no
//! group given has its own (passwd's); its supplementary groups are every group that lists it. A number that isn't in
//! the files is taken as it is, with group 0, as Docker takes it.

use crate::error::{Error, Result};

/// Who runs, and their home folder (passwd's, or `/` for a user it doesn't have).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub user: lambda::User,
    pub home: String,
}

/// A user as `spec` names it, from `passwd`'s and `group`'s text. Empty is root.
pub(crate) fn resolve(passwd: &str, group: &str, spec: &str) -> Result<Resolved> {
    let (name, group_spec) = match spec.split_once(':') {
        Some((u, g)) => (u, Some(g)),
        None => (spec, None),
    };
    let name = if name.is_empty() { "root" } else { name };
    let users: Vec<Vec<&str>> = passwd.lines().map(|l| l.split(':').collect::<Vec<_>>()).filter(|f| f.len() >= 7).collect();
    let groups: Vec<Vec<&str>> = group.lines().map(|l| l.split(':').collect::<Vec<_>>()).filter(|f| f.len() >= 3).collect();
    let entry = users.iter().find(|f| f[0] == name).or_else(|| name.parse::<u32>().ok().and_then(|n| users.iter().find(|f| f[2].parse() == Ok(n))));
    let (uid, primary, home, login) = match entry {
        Some(f) => (f[2].parse().map_err(|_| Error::invalid(format!("/etc/passwd's line for {name} is corrupt")))?, f[3].parse().unwrap_or(0), f[5].to_string(), f[0].to_string()),
        None => match name.parse::<u32>() {
            Ok(uid) => (uid, 0, if uid == 0 { "/root".into() } else { "/".into() }, String::new()),
            Err(_) => return Err(Error::not_found(format!("unable to find user {name}: no matching entries in the image's /etc/passwd"))),
        },
    };
    let gid = match group_spec.filter(|g| !g.is_empty()) {
        None => primary,
        Some(g) => match groups.iter().find(|f| f[0] == g) {
            Some(f) => f[2].parse().unwrap_or(0),
            None => g.parse().map_err(|_| Error::not_found(format!("unable to find group {g}: no matching entries in the image's /etc/group")))?,
        },
    };
    // Supplementary groups: those that list the user (by name), and its own group; none for a group given explicitly.
    let mut supplementary: Vec<u32> = if group_spec.is_some() || login.is_empty() {
        Vec::new()
    } else {
        groups.iter().filter(|f| f.get(3).is_some_and(|members| members.split(',').any(|m| m == login))).filter_map(|f| f[2].parse().ok()).collect()
    };
    if !supplementary.is_empty() && !supplementary.contains(&gid) {
        supplementary.insert(0, gid);
    }
    supplementary.dedup();
    Ok(Resolved { user: lambda::User { uid, gid, groups: supplementary }, home })
}

/// A user as `spec` names it, from a machine's disk.
pub(crate) fn from_disk(disk: &lambda::Disk, spec: &str) -> Result<Resolved> {
    let read = |path: &str| disk.resolve(layers::ROOT, path, true).and_then(|ino| disk.contents(ino)).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    resolve(&read("/etc/passwd"), &read("/etc/group"), spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\nnode:x:1000:1000::/home/node:/bin/sh\npostgres:x:999:999::/var/lib/postgresql:/bin/bash\n";
    const GROUP: &str = "root:x:0:\nnode:x:1000:\npostgres:x:999:\nssl-cert:x:101:postgres\nwheel:x:10:node,someone\n";

    fn user(spec: &str) -> Result<Resolved> {
        resolve(PASSWD, GROUP, spec)
    }

    #[test]
    fn users_as_docker_reads_them() {
        let r = user("node").unwrap();
        assert_eq!((r.user.uid, r.user.gid, r.home.as_str()), (1000, 1000, "/home/node"));
        assert_eq!(r.user.groups, [1000, 10], "its own group, then those listing it");
        assert_eq!(user("postgres").unwrap().user.groups, [999, 101]);
        assert_eq!(user("1000").unwrap().user.uid, 1000);
        assert_eq!(user("node:wheel").unwrap().user.gid, 10);
        assert_eq!(user("node:50").unwrap().user.gid, 50);
        let unknown = user("4242:4242").unwrap();
        assert_eq!((unknown.user.uid, unknown.user.gid, unknown.home.as_str()), (4242, 4242, "/"));
        assert_eq!(user("4242").unwrap().user.gid, 0, "a number it hasn't got, with group 0");
        assert_eq!(user("").unwrap().user, lambda::User { uid: 0, gid: 0, groups: vec![] });
        assert!(user("nobody-here").is_err());
        assert!(user("node:nogroup").is_err());
    }
}
