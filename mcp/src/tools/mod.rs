//! The tools: each its schema, what it is (annotations), and its call into the library.

pub(crate) mod events;
mod files;
mod images;
mod machines;
pub(crate) mod processes;
mod screens;

use serde_json::{Map, Value, json};

use crate::shape::{Args, Failure, Outcome};
use crate::{Progress, Server};

/// What a tool is, as its annotations say, so a client knows which to ask the user about.
#[derive(Clone, Copy)]
pub(crate) enum Is {
    ReadOnly,
    Changes,
    Destructive,
    Idempotent,
    OpenWorld,
}

type Handler = fn(&Server, &Args, &Progress) -> Outcome;

pub(crate) struct Tool {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// Its arguments' properties, and which are needed, but `machine`.
    pub properties: Value,
    pub required: &'static [&'static str],
    pub is: Is,
    /// It acts on one machine: it takes `machine`, and it's listed when the server serves one machine.
    pub on_machine: bool,
    /// It names host paths: listed only when the server has host folders.
    pub host: bool,
    pub run: Handler,
}

impl Tool {
    fn definition(&self, one_machine: bool) -> Value {
        let mut properties = self.properties.as_object().cloned().unwrap_or_default();
        let mut required: Vec<&str> = self.required.to_vec();
        if self.on_machine && !one_machine {
            let mut with = Map::new();
            with.insert("machine".into(), json!({ "type": "string", "description": "The machine, by name" }));
            with.extend(properties);
            properties = with;
            required.insert(0, "machine");
        }
        let (read_only, destructive, idempotent, open_world) = match self.is {
            Is::ReadOnly => (true, false, true, false),
            Is::Changes => (false, false, false, false),
            Is::Destructive => (false, true, false, false),
            Is::Idempotent => (false, false, true, false),
            Is::OpenWorld => (false, false, false, true),
        };
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": { "type": "object", "properties": properties, "required": required, "additionalProperties": false },
            "annotations": { "title": self.title, "readOnlyHint": read_only, "destructiveHint": destructive, "idempotentHint": idempotent, "openWorldHint": open_world },
        })
    }
}

/// A machine's record, as the tools and resources give it.
pub(crate) fn machine_json(info: &carbon::MachineInfo) -> Value {
    machines::info_json(info)
}

pub(crate) fn all() -> Vec<Tool> {
    let mut tools = Vec::new();
    tools.extend(machines::tools());
    tools.extend(processes::tools());
    tools.extend(files::tools());
    tools.extend(images::tools());
    tools.extend(screens::tools());
    tools.extend(events::tools());
    tools
}

impl Server {
    /// The tools this server lists: for one machine, that machine's; with no host folders, none that name host paths;
    /// with `--tools`, those.
    fn listed(&self) -> Vec<Tool> {
        all()
            .into_iter()
            .filter(|t| self.options.machine.is_none() || t.on_machine)
            .filter(|t| !t.host || !self.host_dirs.is_empty())
            .filter(|t| self.options.tools.as_ref().is_none_or(|only| only.iter().any(|n| n == t.name)))
            .collect()
    }

    pub(crate) fn tool_list(&self) -> Vec<Value> {
        let one = self.options.machine.is_some();
        self.listed().iter().map(|t| t.definition(one)).collect()
    }

    pub(crate) fn call_tool(&self, name: &str, args: &Value, progress: &Progress) -> Result<Value, (i64, String)> {
        let Some(tool) = self.listed().into_iter().find(|t| t.name == name) else {
            return Err((-32602, format!("{name} isn't a tool this server lists")));
        };
        let mut args = args.clone();
        if !args.is_object() {
            args = json!({});
        }
        if let Some(m) = &self.options.machine {
            // A call naming another machine is refused, not quietly run in this one.
            if let Some(other) = args.get("machine").and_then(Value::as_str).filter(|o| *o != m)
                && tool.on_machine
            {
                return Ok(crate::shape::failed(&format!("this server serves {m} alone, not {other}")));
            }
            args["machine"] = json!(m);
        }
        Ok(match (tool.run)(self, &Args(&args), progress) {
            Ok(result) => result,
            Err(Failure(message)) => crate::shape::failed(&message),
        })
    }

    /// The machine a call names.
    pub(crate) fn machine(&self, args: &Args) -> Result<carbon::Machine, Failure> {
        let name = args.str("machine")?;
        if let Some(only) = &self.options.machine
            && *only != name
        {
            return Err(Failure(format!("this server serves {only} alone")));
        }
        Ok(self.carbon.get(&name)?)
    }
}

/// A schema property: its type and what it is.
pub(crate) fn prop(kind: &str, description: &str) -> Value {
    json!({ "type": kind, "description": description })
}

/// A command: a string run by the machine's shell, or an array run as it is.
pub(crate) fn command_prop(description: &str) -> Value {
    json!({ "description": description, "anyOf": [{ "type": "string" }, { "type": "array", "items": { "type": "string" } }] })
}

pub(crate) fn strings_prop(description: &str) -> Value {
    json!({ "type": "array", "items": { "type": "string" }, "description": description })
}

pub(crate) fn env_prop() -> Value {
    json!({ "type": "object", "additionalProperties": { "type": "string" }, "description": "Environment variables, over the image's" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_is_named_once_and_described() {
        let tools = all();
        let mut names: Vec<&str> = tools.iter().map(|t| t.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "a name twice");
        for t in &tools {
            assert!(!t.description.is_empty() && !t.title.is_empty(), "{}", t.name);
            let d = t.definition(false);
            for r in d["inputSchema"]["required"].as_array().unwrap() {
                assert!(d["inputSchema"]["properties"].get(r.as_str().unwrap()).is_some(), "{} needs {r}, which it doesn't describe", t.name);
            }
            let one = t.definition(true);
            assert!(one["inputSchema"]["properties"].get("machine").is_none() || !t.on_machine, "{}", t.name);
        }
    }
}
