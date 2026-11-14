//! Writes `gpu/wire/src/vk.rs`: Vulkan's types, constants and command signatures, from Khronos's registry
//! (`vk.xml`), for the commands the guest driver implements and everything they reference.
//!
//! ```sh
//! cargo run -p gpu-wire-gen -- path/to/vk.xml > gpu/wire/src/vk.rs
//! ```
//!
//! The registry is Vulkan-Headers' `registry/vk.xml`; the version the file was written from is in its first line.
//! Only what Vulkan (not Vulkan SC) declares is read. Structs, unions, handles, enums and bitmasks become
//! `#[repr(C)]` Rust in the C ABI's layout; each enum type's values, the core's and every extension's, become
//! constants; each command becomes a `PFN_` type.

mod xml;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write;

use xml::Element;

/// The commands the guest driver implements (or the loader asks it for): their types are what's written.
const COMMANDS: &[&str] = &[
    "vkCreateInstance",
    "vkDestroyInstance",
    "vkEnumerateInstanceExtensionProperties",
    "vkEnumerateInstanceLayerProperties",
    "vkEnumerateInstanceVersion",
    "vkEnumeratePhysicalDevices",
    "vkGetInstanceProcAddr",
    "vkGetDeviceProcAddr",
    "vkGetPhysicalDeviceProperties",
    "vkGetPhysicalDeviceProperties2",
    "vkGetPhysicalDeviceFeatures",
    "vkGetPhysicalDeviceFeatures2",
    "vkGetPhysicalDeviceMemoryProperties",
    "vkGetPhysicalDeviceMemoryProperties2",
    "vkGetPhysicalDeviceQueueFamilyProperties",
    "vkGetPhysicalDeviceQueueFamilyProperties2",
    "vkGetPhysicalDeviceFormatProperties",
    "vkGetPhysicalDeviceFormatProperties2",
    "vkGetPhysicalDeviceImageFormatProperties",
    "vkGetPhysicalDeviceImageFormatProperties2",
    "vkGetPhysicalDeviceSparseImageFormatProperties",
    "vkGetPhysicalDeviceSparseImageFormatProperties2",
    "vkEnumerateDeviceExtensionProperties",
    "vkEnumerateDeviceLayerProperties",
    "vkCreateDevice",
    "vkDestroyDevice",
    "vkGetDeviceQueue",
    "vkDeviceWaitIdle",
    "vkCreateImage",
    "vkDestroyImage",
    "vkGetImageMemoryRequirements",
    "vkCreateBuffer",
    "vkDestroyBuffer",
    "vkGetBufferMemoryRequirements",
    "vkAllocateMemory",
    "vkFreeMemory",
    "vkMapMemory",
    "vkUnmapMemory",
    "vkFlushMappedMemoryRanges",
    "vkInvalidateMappedMemoryRanges",
    "vkBindBufferMemory",
    "vkBindImageMemory",
    "vkGetDeviceMemoryCommitment",
    "vkGetImageSubresourceLayout",
    "vkGetImageSparseMemoryRequirements",
    "vkCreateShaderModule",
    "vkDestroyShaderModule",
    "vkCreatePipelineCache",
    "vkDestroyPipelineCache",
    "vkGetPipelineCacheData",
    "vkMergePipelineCaches",
    "vkCreateComputePipelines",
    "vkDestroyPipeline",
    "vkCreatePipelineLayout",
    "vkDestroyPipelineLayout",
    "vkCreateDescriptorSetLayout",
    "vkDestroyDescriptorSetLayout",
    "vkCreateDescriptorPool",
    "vkDestroyDescriptorPool",
    "vkResetDescriptorPool",
    "vkAllocateDescriptorSets",
    "vkFreeDescriptorSets",
    "vkUpdateDescriptorSets",
    "vkCreateCommandPool",
    "vkDestroyCommandPool",
    "vkResetCommandPool",
    "vkAllocateCommandBuffers",
    "vkFreeCommandBuffers",
    "vkBeginCommandBuffer",
    "vkEndCommandBuffer",
    "vkResetCommandBuffer",
    "vkCmdBindPipeline",
    "vkCmdBindDescriptorSets",
    "vkCmdPushConstants",
    "vkCmdDispatch",
    "vkCmdCopyBuffer",
    "vkCmdFillBuffer",
    "vkCmdUpdateBuffer",
    "vkCmdPipelineBarrier",
    "vkQueueSubmit",
    "vkQueueWaitIdle",
    "vkCreateFence",
    "vkDestroyFence",
    "vkResetFences",
    "vkGetFenceStatus",
    "vkWaitForFences",
    "vkCreateSemaphore",
    "vkDestroySemaphore",
    "vkCreateEvent",
    "vkDestroyEvent",
    "vkGetEventStatus",
    "vkSetEvent",
    "vkResetEvent",
    "vkCreateImageView",
    "vkDestroyImageView",
    "vkCreateSampler",
    "vkDestroySampler",
    "vkCreateRenderPass",
    "vkDestroyRenderPass",
    "vkGetRenderAreaGranularity",
    "vkCreateFramebuffer",
    "vkDestroyFramebuffer",
    "vkCreateGraphicsPipelines",
    "vkCmdBeginRenderPass",
    "vkCmdEndRenderPass",
    "vkCmdNextSubpass",
    "vkCmdSetViewport",
    "vkCmdSetScissor",
    "vkCmdBindVertexBuffers",
    "vkCmdBindIndexBuffer",
    "vkCmdDraw",
    "vkCmdDrawIndexed",
    "vkCmdCopyBufferToImage",
    "vkCmdCopyImageToBuffer",
    "vkCmdSetLineWidth",
    "vkCmdSetDepthBias",
    "vkCmdSetBlendConstants",
    "vkCmdSetDepthBounds",
    "vkCmdSetStencilCompareMask",
    "vkCmdSetStencilWriteMask",
    "vkCmdSetStencilReference",
    "vkDestroySurfaceKHR",
    "vkGetPhysicalDeviceSurfaceSupportKHR",
    "vkGetPhysicalDeviceSurfaceCapabilitiesKHR",
    "vkGetPhysicalDeviceSurfaceFormatsKHR",
    "vkGetPhysicalDeviceSurfacePresentModesKHR",
    "vkCreateSwapchainKHR",
    "vkDestroySwapchainKHR",
    "vkGetSwapchainImagesKHR",
    "vkAcquireNextImageKHR",
    "vkQueuePresentKHR",
];

/// Structs read or filled through a `pNext` chain, which no command names.
const STRUCTS: &[&str] = &["VkPhysicalDeviceIDProperties", "VkPhysicalDeviceDriverProperties", "VkBaseOutStructure", "VkBaseInStructure"];

struct Registry<'a> {
    types: BTreeMap<String, &'a Element>,
    /// Each enum type's values, in order: name, value (or an alias's target).
    values: BTreeMap<String, Vec<(String, Value)>>,
    constants: Vec<(String, String, String)>,
    commands: BTreeMap<String, &'a Element>,
    aliases: BTreeMap<String, String>,
}

#[derive(Clone)]
enum Value {
    Int(i64),
    Alias(String),
}

/// Whether an element is Vulkan's (not only Vulkan SC's).
fn vulkan(e: &Element) -> bool {
    e.attr("api").is_none_or(|api| api.split(',').any(|a| a == "vulkan"))
}

fn extension_value(e: &Element, extnumber: Option<i64>) -> Option<Value> {
    if let Some(alias) = e.attr("alias") {
        return Some(Value::Alias(alias.into()));
    }
    if let Some(v) = e.attr("value") {
        return parse_int(v).map(Value::Int);
    }
    if let Some(bit) = e.attr("bitpos") {
        return bit.parse::<u32>().ok().map(|b| Value::Int(1i64 << b));
    }
    let offset: i64 = e.attr("offset")?.parse().ok()?;
    let ext = e.attr("extnumber").and_then(|n| n.parse().ok()).or(extnumber)?;
    let v = 1_000_000_000 + (ext - 1) * 1000 + offset;
    Some(Value::Int(if e.attr("dir") == Some("-") { -v } else { v }))
}

fn parse_int(v: &str) -> Option<i64> {
    let v = v.trim_start_matches('(').trim_end_matches(')').trim_end_matches(['U', 'L', 'F', 'f']);
    if let Some(hex) = v.strip_prefix("0x") {
        return i64::from_str_radix(hex, 16).ok();
    }
    v.parse().ok()
}

impl<'a> Registry<'a> {
    fn read(root: &'a Element) -> Registry<'a> {
        let mut r = Registry { types: BTreeMap::new(), values: BTreeMap::new(), constants: Vec::new(), commands: BTreeMap::new(), aliases: BTreeMap::new() };
        for types in root.children_named("types") {
            for t in types.children_named("type").filter(|t| vulkan(t)) {
                let name = t
                    .attr("name")
                    .map(String::from)
                    .or_else(|| t.child("name").map(|n| n.text()))
                    .or_else(|| t.child("proto").and_then(|p| p.child("name")).map(|n| n.text()));
                let Some(name) = name else { continue };
                if let Some(alias) = t.attr("alias") {
                    r.aliases.insert(name, alias.into());
                } else {
                    r.types.insert(name, t);
                }
            }
        }
        for enums in root.children_named("enums") {
            let Some(name) = enums.attr("name") else { continue };
            if name == "API Constants" {
                for e in enums.children_named("enum") {
                    let (Some(n), ty) = (e.attr("name"), e.attr("type").unwrap_or("uint32_t")) else { continue };
                    let value = e.attr("value").or(e.attr("alias")).unwrap_or("0");
                    r.constants.push((n.into(), ty.into(), value.into()));
                }
                continue;
            }
            let list = r.values.entry(name.into()).or_default();
            for e in enums.children_named("enum").filter(|e| vulkan(e)) {
                if let (Some(n), Some(v)) = (e.attr("name"), extension_value(e, None)) {
                    list.push((n.into(), v));
                }
            }
        }
        // Values the core versions and the extensions add to enum types.
        let mut adds = Vec::new();
        for feature in root.children_named("feature").filter(|f| vulkan(f)) {
            for req in feature.children_named("require") {
                adds.extend(req.children_named("enum").map(|e| (e, None)));
            }
        }
        for exts in root.children_named("extensions") {
            for ext in exts.children_named("extension") {
                if !ext.attr("supported").is_some_and(|s| s.split(',').any(|a| a == "vulkan")) {
                    continue;
                }
                let number = ext.attr("number").and_then(|n| n.parse().ok());
                for req in ext.children_named("require").filter(|q| vulkan(q)) {
                    adds.extend(req.children_named("enum").map(|e| (e, number)));
                }
            }
        }
        for (e, number) in adds {
            let (Some(ty), Some(name)) = (e.attr("extends"), e.attr("name")) else { continue };
            if !vulkan(e) {
                continue;
            }
            let Some(v) = extension_value(e, number) else { continue };
            let list = r.values.entry(ty.into()).or_default();
            if !list.iter().any(|(n, _)| n == name) {
                list.push((name.into(), v));
            }
        }
        for commands in root.children_named("commands") {
            for c in commands.children_named("command").filter(|c| vulkan(c)) {
                if let (Some(name), Some(alias)) = (c.attr("name"), c.attr("alias")) {
                    r.aliases.insert(name.into(), alias.into());
                } else if let Some(name) = c.child("proto").and_then(|p| p.child("name")).map(|n| n.text()) {
                    r.commands.insert(name, c);
                }
            }
        }
        r
    }

    fn resolve<'n>(&'n self, mut name: &'n str) -> &'n str {
        while let Some(a) = self.aliases.get(name) {
            name = a;
        }
        name
    }
}

/// A declaration's parts: `const <type>T</type> * const * <name>n</name>[N]`.
#[derive(Clone)]
struct Decl {
    ty: String,
    name: String,
    /// Before the type (`const`) and between it and the name (`*`, `const`).
    before: String,
    after_type: String,
    /// `[N]` dimensions, each a number or a constant's name.
    dims: Vec<String>,
    bitfield: bool,
}

fn decl(e: &Element) -> Decl {
    let mut d = Decl { ty: String::new(), name: String::new(), before: String::new(), after_type: String::new(), dims: Vec::new(), bitfield: false };
    let mut stage = 0;
    let mut tail = String::new();
    for n in &e.children {
        match n {
            xml::Node::Text(t) => match stage {
                0 => d.before.push_str(t),
                1 => d.after_type.push_str(t),
                _ => tail.push_str(t),
            },
            xml::Node::Element(c) if c.name == "type" => {
                d.ty = c.text();
                stage = 1;
            }
            xml::Node::Element(c) if c.name == "name" => {
                d.name = c.text();
                stage = 2;
            }
            xml::Node::Element(c) if c.name == "enum" => tail.push_str(&c.text()),
            _ => {}
        }
    }
    d.bitfield = tail.trim_start().starts_with(':');
    let mut rest = tail.as_str();
    while let Some(open) = rest.find('[') {
        let close = rest[open..].find(']').map_or(rest.len(), |c| open + c);
        d.dims.push(rest[open + 1..close].trim().to_string());
        rest = &rest[close.min(rest.len() - 1) + 1..];
    }
    d
}

fn base_type(ty: &str) -> String {
    match ty {
        "void" => "c_void".into(),
        "char" => "c_char".into(),
        "float" => "f32".into(),
        "double" => "f64".into(),
        "uint8_t" => "u8".into(),
        "int8_t" => "i8".into(),
        "uint16_t" => "u16".into(),
        "int16_t" => "i16".into(),
        "uint32_t" => "u32".into(),
        "int32_t" | "int" => "i32".into(),
        "uint64_t" => "u64".into(),
        "int64_t" => "i64".into(),
        "size_t" => "usize".into(),
        other => other.into(),
    }
}

/// A parameter's Rust type: an array parameter (`const float blendConstants[4]`) is a pointer to its first element, as
/// C passes it.
fn param_type(d: &Decl) -> String {
    if d.dims.is_empty() {
        return rust_type(d);
    }
    let element = Decl { dims: Vec::new(), ..d.clone() };
    format!("{} {}", if d.before.contains("const") { "*const" } else { "*mut" }, rust_type(&element))
}

/// A declaration's Rust type, pointers built from the inside out.
fn rust_type(d: &Decl) -> String {
    let mut t = base_type(&d.ty);
    let mut const_next = d.before.contains("const");
    let after = d.after_type.as_str();
    let stars: Vec<usize> = after.match_indices('*').map(|(i, _)| i).collect();
    for (k, &at) in stars.iter().enumerate() {
        t = format!("{} {t}", if const_next { "*const" } else { "*mut" });
        let end = stars.get(k + 1).copied().unwrap_or(after.len());
        const_next = after[at + 1..end].contains("const");
    }
    for dim in d.dims.iter().rev() {
        t = format!("[{t}; {dim} as usize]");
    }
    t
}

/// The names a type or command refers to.
fn referenced(e: &Element) -> Vec<String> {
    let mut out = Vec::new();
    for c in e.descendants() {
        if c.name == "type" {
            out.push(c.text());
        }
    }
    if let Some(req) = e.attr("requires") {
        out.push(req.into());
    }
    if let Some(bits) = e.attr("bitvalues") {
        out.push(bits.into());
    }
    out
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: gpu-wire-gen path/to/vk.xml > gpu/wire/src/vk.rs");
        std::process::exit(2);
    };
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(1)
    });
    let root = xml::parse(&text).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(1)
    });
    let r = Registry::read(&root);
    let version = r.types.get("VK_HEADER_VERSION").map(|t| t.text()).unwrap_or_default();
    let version = version.split_whitespace().last().unwrap_or("?").to_string();

    // Everything the commands and extra structs reach.
    let mut want: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    for c in COMMANDS {
        let c = r.resolve(c);
        let Some(cmd) = r.commands.get(c) else { panic!("{c} isn't in the registry") };
        queue.extend(referenced(cmd));
    }
    queue.extend(STRUCTS.iter().map(|s| s.to_string()));
    queue.extend(["VkStructureType", "VkResult", "VkBool32", "VkFlags", "VkFlags64", "VkDeviceSize"].map(String::from));
    while let Some(name) = queue.pop_front() {
        let name = r.resolve(&name).to_string();
        if !want.insert(name.clone()) {
            continue;
        }
        if let Some(t) = r.types.get(&name) {
            queue.extend(referenced(t));
            // A bitmask's bits type.
            if t.attr("category") == Some("bitmask") {
                let bits = name.replace("Flags", "FlagBits");
                queue.push_back(bits);
            }
        }
    }

    let mut out = String::new();
    let _ = writeln!(out, "//! Vulkan's types for the guest driver's commands, generated from `vk.xml` (header version {version}) by");
    out.push_str("//! `gpu-wire-gen`: not edited by hand.\n\n#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals, dead_code, clippy::all)]\n\n");
    out.push_str("use core::ffi::{c_char, c_void};\n\n");
    for (name, ty, value) in &r.constants {
        let rust = match ty.as_str() {
            "float" => "f32",
            "uint64_t" => "u64",
            "int32_t" => "i32",
            _ => "u32",
        };
        let value = if let Some(v) = parse_int(value) {
            if rust == "u32" || rust == "u64" { format!("{}", v as u64 & if rust == "u32" { 0xffff_ffff } else { u64::MAX }) } else { v.to_string() }
        } else if rust == "f32" {
            value.trim_end_matches(['F', 'f']).to_string()
        } else {
            match value.as_str() {
                "(~0U)" => "!0".into(),
                "(~1U)" => "!1".into(),
                "(~2U)" => "!2".into(),
                "(~0ULL)" => "!0".into(),
                other => other.into(),
            }
        };
        let _ = writeln!(out, "pub const {name}: {rust} = {value};");
    }
    out.push('\n');
    for name in &want {
        let Some(t) = r.types.get(name) else { continue };
        let category = t.attr("category").unwrap_or("");
        match category {
            "basetype" => {
                let d = decl(t);
                if d.ty.is_empty() {
                    let _ = writeln!(out, "pub type {name} = *mut c_void;");
                } else {
                    let _ = writeln!(out, "pub type {name} = {};", rust_type(&d));
                }
            }
            "bitmask" => {
                let wide = t.descendants().into_iter().any(|c| c.name == "type" && c.text() == "VkFlags64");
                let _ = writeln!(out, "pub type {name} = {};", if wide { "u64" } else { "u32" });
            }
            "handle" => {
                let dispatchable = t.descendants().into_iter().any(|c| c.name == "type" && c.text() == "VK_DEFINE_HANDLE");
                let _ = writeln!(out, "pub type {name} = {};", if dispatchable { "*mut c_void" } else { "u64" });
            }
            "enum" => {
                let wide = r.values.contains_key(name) && name.contains("FlagBits2");
                let ty = if name.contains("FlagBits") { if wide { "u64" } else { "u32" } } else { "i32" };
                let _ = writeln!(out, "pub type {name} = {ty};");
                for (n, v) in r.values.get(name).into_iter().flatten() {
                    match v {
                        Value::Int(i) if ty == "i32" => {
                            let _ = writeln!(out, "pub const {n}: {name} = {i};");
                        }
                        Value::Int(i) => {
                            let _ = writeln!(out, "pub const {n}: {name} = {:#x};", *i as u64 & if ty == "u32" { 0xffff_ffff } else { u64::MAX });
                        }
                        Value::Alias(a) => {
                            let _ = writeln!(out, "pub const {n}: {name} = {a};");
                        }
                    }
                }
            }
            "funcpointer" => {
                let _ = writeln!(out, "pub type {name} = Option<unsafe extern \"C\" fn()>;");
            }
            "struct" | "union" => {
                let members: Vec<Decl> = t.children_named("member").filter(|m| vulkan(m)).map(decl).collect();
                if members.iter().any(|m| m.bitfield) {
                    // Bitfields have no Rust layout: the struct is skipped, and nothing here uses one.
                    continue;
                }
                let keyword = if category == "union" { "union" } else { "struct" };
                let _ = writeln!(out, "#[repr(C)]\n#[derive(Clone, Copy)]\npub {keyword} {name} {{");
                for m in &members {
                    let field = if m.name == "type" { "type_".to_string() } else { m.name.clone() };
                    let _ = writeln!(out, "    pub {field}: {},", rust_type(m));
                }
                out.push_str("}\n");
            }
            _ => {}
        }
    }
    for (alias, target) in &r.aliases {
        if want.contains(target) && r.types.contains_key(target) && !alias.starts_with("vk") {
            let _ = writeln!(out, "pub type {alias} = {target};");
        }
    }
    out.push('\n');
    for c in COMMANDS {
        let real = r.resolve(c);
        let cmd = r.commands[real];
        let proto = decl(cmd.child("proto").expect("a command's proto"));
        let params: Vec<String> = cmd
            .children_named("param")
            .filter(|p| vulkan(p))
            .map(|p| {
                let d = decl(p);
                let name = if d.name == "type" { "type_" } else { d.name.as_str() };
                format!("{name}: {}", param_type(&d))
            })
            .collect();
        let ret = if proto.ty == "void" { String::new() } else { format!(" -> {}", base_type(&proto.ty)) };
        let _ = writeln!(out, "pub type PFN_{c} = unsafe extern \"C\" fn({}){ret};", params.join(", "));
    }
    print!("{out}");
}
