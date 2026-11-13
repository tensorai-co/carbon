//! Window stations and desktops: `WinSta0` and its `Default`, which every process starts on, and those programs make
//! (a sandbox makes a station and a desktop of its own for the processes it confines). They're user objects with
//! names, a station each desktop is in, and a security descriptor (kept by `secapi`). Every desktop's windows still go
//! on the machine's one screen: a confined process doesn't show any.

use std::collections::{BTreeMap, HashMap};

/// `WinSta0` and its `Default` desktop: the handles every process has for them, from `GetProcessWindowStation` and
/// `GetThreadDesktop`.
pub const WINDOW_STATION: u64 = 0x7f00;
pub const DESKTOP: u64 = 0x7f04;

/// One window station or desktop.
#[derive(Clone, Debug)]
pub struct UserObject {
    pub name: String,
    /// A desktop's station; 0 for a station.
    pub station: u64,
}

impl UserObject {
    pub fn is_station(&self) -> bool {
        self.station == 0
    }
}

pub struct Stations {
    objects: BTreeMap<u64, UserObject>,
    /// Each process's window station, by process ID, where it isn't `WinSta0`.
    current: HashMap<u64, u64>,
    next: u64,
}

impl Default for Stations {
    fn default() -> Stations {
        let mut objects = BTreeMap::new();
        objects.insert(WINDOW_STATION, UserObject { name: "WinSta0".into(), station: 0 });
        objects.insert(DESKTOP, UserObject { name: "Default".into(), station: WINDOW_STATION });
        Stations { objects, current: HashMap::new(), next: DESKTOP + 4 }
    }
}

impl Stations {
    pub fn get(&self, handle: u64) -> Option<&UserObject> {
        self.objects.get(&handle)
    }

    /// A window station by name (any case), made if there isn't one and `create`; none, `Service-0x0-N$` as a
    /// service's are named. `Err(true)` if `only_new` and it's there already, `Err(false)` if it isn't and can't be.
    pub fn station(&mut self, name: Option<&str>, create: bool, only_new: bool) -> Result<u64, bool> {
        let name = match name {
            Some(n) if !n.is_empty() => n.to_string(),
            _ if create => format!("Service-0x0-{:x}$", self.next),
            _ => return Err(false),
        };
        self.find(&name, 0, only_new).or_else(|e| if create && !e { Ok(self.add(name, 0)) } else { Err(e) })
    }

    /// A desktop by name in `station`, made if there isn't one and `create`.
    pub fn desktop(&mut self, station: u64, name: &str, create: bool) -> Result<u64, bool> {
        self.find(name, station, false).or_else(|e| if create && !e && !name.is_empty() { Ok(self.add(name.to_string(), station)) } else { Err(e) })
    }

    fn find(&self, name: &str, station: u64, only_new: bool) -> Result<u64, bool> {
        let found = self.objects.iter().find(|(_, o)| o.station == station && o.name.eq_ignore_ascii_case(name)).map(|(h, _)| *h);
        match found {
            Some(_) if only_new => Err(true),
            Some(h) => Ok(h),
            None => Err(false),
        }
    }

    fn add(&mut self, name: String, station: u64) -> u64 {
        let handle = self.next;
        self.next += 4;
        self.objects.insert(handle, UserObject { name, station });
        handle
    }

    /// Closes one a program made (and a station's desktops with it); `WinSta0` and `Default` stay. Whether it was one.
    pub fn close(&mut self, handle: u64, station: bool) -> bool {
        match self.objects.get(&handle) {
            Some(o) if o.is_station() == station => {}
            _ => return false,
        }
        if handle != WINDOW_STATION && handle != DESKTOP {
            self.objects.remove(&handle);
            self.objects.retain(|_, o| o.station != handle);
            self.current.retain(|_, s| *s != handle);
        }
        true
    }

    /// The process's window station.
    pub fn current(&self, pid: u64) -> u64 {
        self.current.get(&pid).copied().unwrap_or(WINDOW_STATION)
    }

    pub fn set_current(&mut self, pid: u64, station: u64) -> bool {
        if !self.objects.get(&station).is_some_and(UserObject::is_station) {
            return false;
        }
        if station == WINDOW_STATION {
            self.current.remove(&pid);
        } else {
            self.current.insert(pid, station);
        }
        true
    }

    /// A process that's over has no station.
    pub fn forget(&mut self, pid: u64) {
        self.current.remove(&pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sandbox_station_and_desktop() {
        let mut s = Stations::default();
        let station = s.station(None, true, false).unwrap();
        assert!(s.get(station).unwrap().name.starts_with("Service-0x0-"));
        assert!(s.set_current(7, station));
        let desktop = s.desktop(s.current(7), "sbox_alternate_desktop", true).unwrap();
        assert_eq!(s.get(desktop).unwrap().station, station);
        // The same name in another station is another desktop; asking again finds the same one.
        assert_ne!(s.desktop(WINDOW_STATION, "sbox_alternate_desktop", true).unwrap(), desktop);
        assert_eq!(s.desktop(station, "SBOX_alternate_desktop", false), Ok(desktop));
        assert_eq!(s.station(Some(&s.get(station).unwrap().name.clone()), true, true), Err(true));
        // Closing the station takes its desktop, and the process goes back to WinSta0.
        assert!(s.close(station, true));
        assert!(s.get(desktop).is_none());
        assert_eq!(s.current(7), WINDOW_STATION);
        assert!(s.close(WINDOW_STATION, true) && s.get(WINDOW_STATION).is_some());
        assert!(!s.close(DESKTOP, true));
    }
}
