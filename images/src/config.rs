//! What an image's config says, all of OCI's: what to run, as whom, where, with what; its labels, ports, volumes, stop
//! signal, health check and shell; what to do when built on (`ONBUILD`); and its history, each step that made it. Kept
//! as JSON of Carbon's own (the record of an image, and of a machine made from it), read from a pulled image's OCI
//! config.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

/// What an image's config says about running it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub env: Vec<String>,
    pub entrypoint: Vec<String>,
    pub cmd: Vec<String>,
    pub workdir: String,
    pub user: String,
    pub labels: BTreeMap<String, String>,
    /// The ports it says it listens on: `80/tcp`, `53/udp`.
    pub exposed_ports: Vec<String>,
    /// The folders it says hold data a machine keeps: `/var/lib/postgresql/data`.
    pub volumes: Vec<String>,
    /// What stops it: `SIGTERM` (the default when empty), `SIGQUIT`, `9`.
    pub stop_signal: String,
    /// The shell a shell-form instruction runs under: `/bin/sh -c` when empty.
    pub shell: Vec<String>,
    pub healthcheck: Option<Healthcheck>,
    /// Instructions for a build `FROM` it.
    pub onbuild: Vec<String>,
}

/// How a machine from it is checked to be well: a command, how often, how long it may take, and how many failures
/// make it unwell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Healthcheck {
    /// `["CMD", "curl", "-f", "http://localhost/"]`, `["CMD-SHELL", "..."]`, or `["NONE"]`.
    pub test: Vec<String>,
    /// Nanoseconds; 0 for the default.
    pub interval: u64,
    pub timeout: u64,
    pub start_period: u64,
    pub retries: u32,
}

/// One step of what made an image.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct History {
    /// RFC 3339, as OCI has it.
    pub created: String,
    /// The instruction, or the command, that made it.
    pub created_by: String,
    /// It changed only the config, not the disk.
    pub empty_layer: bool,
    pub comment: String,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
}

/// The keys of an OCI object-as-set (`{"80/tcp": {}}`).
fn keys(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_object).map(|o| o.keys().cloned().collect()).unwrap_or_default()
}

fn text(v: Option<&Value>, key: &str) -> String {
    v.and_then(|v| v.get(key)).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn labels(v: Option<&Value>) -> BTreeMap<String, String> {
    v.and_then(Value::as_object).map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()).unwrap_or_default()
}

impl Config {
    /// As Carbon keeps it.
    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "env": self.env,
            "entrypoint": self.entrypoint,
            "cmd": self.cmd,
            "workdir": self.workdir,
            "user": self.user,
        });
        // What most images don't say is left out, so a record stays as short as it was.
        if !self.labels.is_empty() {
            v["labels"] = json!(self.labels);
        }
        if !self.exposed_ports.is_empty() {
            v["exposed_ports"] = json!(self.exposed_ports);
        }
        if !self.volumes.is_empty() {
            v["volumes"] = json!(self.volumes);
        }
        if !self.stop_signal.is_empty() {
            v["stop_signal"] = json!(self.stop_signal);
        }
        if !self.shell.is_empty() {
            v["shell"] = json!(self.shell);
        }
        if let Some(h) = &self.healthcheck {
            v["healthcheck"] = json!({ "test": h.test, "interval": h.interval, "timeout": h.timeout, "start_period": h.start_period, "retries": h.retries });
        }
        if !self.onbuild.is_empty() {
            v["onbuild"] = json!(self.onbuild);
        }
        v
    }

    /// From [`Config::to_json`]'s JSON (or a record from before it kept more than what to run).
    pub fn from_json(v: &Value) -> Config {
        let u = |h: &Value, k: &str| h.get(k).and_then(Value::as_u64).unwrap_or(0);
        Config {
            env: strings(v.get("env")),
            entrypoint: strings(v.get("entrypoint")),
            cmd: strings(v.get("cmd")),
            workdir: text(Some(v), "workdir"),
            user: text(Some(v), "user"),
            labels: labels(v.get("labels")),
            exposed_ports: strings(v.get("exposed_ports")),
            volumes: strings(v.get("volumes")),
            stop_signal: text(Some(v), "stop_signal"),
            shell: strings(v.get("shell")),
            healthcheck: v.get("healthcheck").filter(|h| h.is_object()).map(|h| Healthcheck {
                test: strings(h.get("test")),
                interval: u(h, "interval"),
                timeout: u(h, "timeout"),
                start_period: u(h, "start_period"),
                retries: u(h, "retries") as u32,
            }),
            onbuild: strings(v.get("onbuild")),
        }
    }

    /// From an OCI image config's `config` object, as a registry gives it.
    pub fn from_oci(run: Option<&Value>) -> Config {
        let h = run.and_then(|r| r.get("Healthcheck")).filter(|h| h.is_object());
        let u = |h: &Value, k: &str| h.get(k).and_then(Value::as_u64).unwrap_or(0);
        Config {
            env: strings(run.and_then(|c| c.get("Env"))),
            entrypoint: strings(run.and_then(|c| c.get("Entrypoint"))),
            cmd: strings(run.and_then(|c| c.get("Cmd"))),
            workdir: text(run, "WorkingDir"),
            user: text(run, "User"),
            labels: labels(run.and_then(|c| c.get("Labels"))),
            exposed_ports: keys(run.and_then(|c| c.get("ExposedPorts"))),
            volumes: keys(run.and_then(|c| c.get("Volumes"))),
            stop_signal: text(run, "StopSignal"),
            shell: strings(run.and_then(|c| c.get("Shell"))),
            healthcheck: h.map(|h| Healthcheck { test: strings(h.get("Test")), interval: u(h, "Interval"), timeout: u(h, "Timeout"), start_period: u(h, "StartPeriod"), retries: u(h, "Retries") as u32 }),
            onbuild: strings(run.and_then(|c| c.get("OnBuild"))),
        }
    }

    /// As an OCI image config's `config` object, for a push.
    pub fn to_oci(&self) -> Value {
        let set = |items: &[String]| Value::Object(items.iter().map(|k| (k.clone(), json!({}))).collect::<Map<_, _>>());
        let mut v = json!({ "Env": self.env, "Cmd": self.cmd });
        let mut put = |k: &str, value: Value| {
            v[k] = value;
        };
        if !self.entrypoint.is_empty() {
            put("Entrypoint", json!(self.entrypoint));
        }
        if !self.workdir.is_empty() {
            put("WorkingDir", json!(self.workdir));
        }
        if !self.user.is_empty() {
            put("User", json!(self.user));
        }
        if !self.labels.is_empty() {
            put("Labels", json!(self.labels));
        }
        if !self.exposed_ports.is_empty() {
            put("ExposedPorts", set(&self.exposed_ports));
        }
        if !self.volumes.is_empty() {
            put("Volumes", set(&self.volumes));
        }
        if !self.stop_signal.is_empty() {
            put("StopSignal", json!(self.stop_signal));
        }
        if !self.shell.is_empty() {
            put("Shell", json!(self.shell));
        }
        if let Some(h) = &self.healthcheck {
            put("Healthcheck", json!({ "Test": h.test, "Interval": h.interval, "Timeout": h.timeout, "StartPeriod": h.start_period, "Retries": h.retries }));
        }
        if !self.onbuild.is_empty() {
            put("OnBuild", json!(self.onbuild));
        }
        v
    }

    /// The stop signal's number: its name (`SIGTERM`, `TERM`) or number, SIGTERM's when it says none.
    pub fn stop_signal_number(&self) -> i32 {
        signal_number(&self.stop_signal).unwrap_or(15)
    }
}

/// A signal's number from its name or number, as Linux numbers them.
pub fn signal_number(text: &str) -> Option<i32> {
    if let Ok(n) = text.parse() {
        return Some(n);
    }
    let name = text.trim().to_ascii_uppercase();
    Some(match name.strip_prefix("SIG").unwrap_or(&name) {
        "HUP" => 1,
        "INT" => 2,
        "QUIT" => 3,
        "KILL" => 9,
        "USR1" => 10,
        "USR2" => 12,
        "TERM" => 15,
        "STOP" => 19,
        "PWR" => 30,
        "WINCH" => 28,
        _ => return None,
    })
}

impl History {
    pub fn to_json(&self) -> Value {
        json!({ "created": self.created, "created_by": self.created_by, "empty_layer": self.empty_layer, "comment": self.comment })
    }

    /// From Carbon's JSON, or an OCI config's `history` entry (whose keys are the same).
    pub fn from_json(v: &Value) -> History {
        History { created: text(Some(v), "created"), created_by: text(Some(v), "created_by"), empty_layer: v.get("empty_layer").and_then(Value::as_bool).unwrap_or(false), comment: text(Some(v), "comment") }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oci_config_is_read_kept_and_written_back() {
        let oci = json!({
            "Env": ["PATH=/bin"], "Cmd": ["nginx", "-g", "daemon off;"], "Entrypoint": ["/docker-entrypoint.sh"],
            "ExposedPorts": { "80/tcp": {} }, "Volumes": { "/data": {} }, "StopSignal": "SIGQUIT",
            "Labels": { "maintainer": "NGINX" }, "Healthcheck": { "Test": ["CMD", "true"], "Interval": 30_000_000_000u64, "Retries": 3 },
            "Shell": ["/bin/bash", "-c"], "OnBuild": ["RUN echo built"]
        });
        let config = Config::from_oci(Some(&oci));
        assert_eq!(config.exposed_ports, vec!["80/tcp".to_string()]);
        assert_eq!(config.stop_signal_number(), 3);
        assert_eq!(config.healthcheck.as_ref().unwrap().retries, 3);
        assert_eq!(Config::from_json(&config.to_json()), config);
        assert_eq!(Config::from_oci(Some(&config.to_oci())), config);
        // A record from before: what to run only.
        let old = json!({ "env": ["A=1"], "cmd": ["bash"], "entrypoint": [], "workdir": "", "user": "" });
        assert_eq!(Config::from_json(&old).cmd, vec!["bash".to_string()]);
        assert_eq!(signal_number("SIGKILL"), Some(9));
        assert_eq!(signal_number("term"), Some(15));
        assert_eq!(signal_number("BOGUS"), None);
    }
}
