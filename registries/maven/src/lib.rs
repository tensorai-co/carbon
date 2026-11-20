//! `registries-maven`: a Maven repository's layout (Maven Central's) as registries' calls.
//!
//! | Maven asks for | The call |
//! | --- | --- |
//! | `/{folder}/maven-metadata.xml` | `metadata {path}`: an artifact's versions, or a group's plugins |
//! | `/{folder}/maven-metadata.xml.sha1` (`.md5`, `.sha256`, `.sha512`) | `metadata {path, checksum}`, of the metadata as the server rebuilt it |
//! | `/{group…}/{artifact}/{version}/{artifact}-{version}[-{classifier}].{ext}` | `file {group, artifact, version, file}` |
//!
//! A file's checksums and signature (`.sha1`, `.asc`) are files too. A file's name must be its version's own
//! (`{artifact}-{version}` and what follows), so no call names anything else. Under `/-plugins/`, the same paths are
//! Maven's plugin repository, and each call says `for: plugins`, so the server can hold what Maven fetches to run its
//! plugins to its own rule. (No group starts with `-`, so the prefix is never a group's folder.)
//!
//! Maven can't be pointed at a repository by a variable alone: it reads them from its settings. So the machine gets a
//! settings file in a tmpfs for the run, `/etc/carbon/maven/settings.xml`, and `MAVEN_ARGS` names it as Maven's global
//! settings, with the wagon transport, which shows a refusal's reason. Its one profile, always active, makes `central` this registry, for dependencies and for plugins, and its
//! mirror sends every other repository a build names here too. A user's own `~/.m2/settings.xml` still applies on top
//! of it.

use std::sync::Arc;

use registries_core::{Adapter, Answer, Call, Conversation, Field, Native, Op, Request, clean, decode_segment};
use serde_json::Value;

const SERVICE: &str = "maven";
/// Where the machine's Maven settings are, in a tmpfs.
pub const SETTINGS: &str = "/etc/carbon/maven/settings.xml";
/// Gradle's init script, in the user's Gradle home, which Gradle runs before every build.
pub const GRADLE_INIT: &str = "~/.gradle/init.d/carbon-registries.gradle";
/// sbt's repositories, which `SBT_OPTS` names.
pub const SBT_REPOSITORIES: &str = "/etc/carbon/sbt/repositories";

fn segment(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && (s.as_bytes()[0].is_ascii_alphanumeric() || s.as_bytes()[0] == b'_') && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b)) && !s.contains("..")
}

/// A folder of the repository: 1 to 32 segments of letters, digits, `_ . -`, none starting with a dot or holding `..`.
pub fn is_path(p: &str) -> bool {
    p.len() <= 512 && p.split('/').count() <= 32 && p.split('/').all(segment)
}

/// A groupId: dotted words of letters, digits, `_` and `-`, none starting with `-`.
pub fn is_group(s: &str) -> bool {
    s.len() <= 512 && s.split('.').count() <= 16 && s.split('.').all(|w| !w.is_empty() && w.len() <= 64 && !w.starts_with('-') && w.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)))
}

/// An artifactId.
pub fn is_artifact(s: &str) -> bool {
    segment(s)
}

/// A release's version: a repository like Central has no snapshots.
pub fn is_version(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && (s.as_bytes()[0].is_ascii_alphanumeric() || s.as_bytes()[0] == b'_') && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.+-".contains(&b)) && !s.contains("..") && !s.ends_with("-SNAPSHOT")
}

/// A file's name, at most 300 bytes, with no `/` or `..`. That it's its version's own is checked when it's parsed.
fn is_file(s: &str) -> bool {
    segment(s) && s.len() <= 300
}

/// What follows `{artifact}-{version}` in a file's name: a classifier (`-sources`, `-linux-x86_64`), then one to three
/// extensions (`.jar`, `.tar.gz`, `.jar.sha1`).
fn is_rest(rest: &str) -> bool {
    let mut parts = rest.split('.');
    let classifier = parts.next().unwrap_or_default();
    let exts: Vec<&str> = parts.collect();
    (classifier.is_empty() || classifier.strip_prefix('-').is_some_and(|c| c.len() <= 64 && segment(c)))
        && (1..=3).contains(&exts.len())
        && exts.iter().all(|e| !e.is_empty() && e.len() <= 16 && e.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// A call for Maven's plugins: what it fetches through its plugin repository.
const FOR: Field = Field::optional("for", |s| s == "plugins");
/// A call for Gradle's plugins, whose repositories are Maven Central and the Gradle Plugin Portal.
const REPO: Field = Field::optional("repo", |s| s == "gradle-plugins");

/// The prefix of Maven's plugin repository's paths.
const PLUGINS: &str = "-plugins";
/// The prefix of Gradle's plugin repository's paths: Maven Central, then the Gradle Plugin Portal.
const GRADLE_PLUGINS: &str = "-gradle-plugins";

static OPS: &[Op] = &[
    Op { name: "metadata", fields: &[Field::required("path", is_path), Field::optional("checksum", |s| matches!(s, "sha1" | "md5" | "sha256" | "sha512")), FOR, REPO] },
    Op { name: "file", fields: &[Field::required("group", is_group), Field::required("artifact", is_artifact), Field::required("version", is_version), Field::required("file", is_file), FOR, REPO] },
];

/// Maven's adapter. It keeps nothing.
pub struct Maven;

impl Maven {
    pub fn adapter() -> Arc<dyn Adapter> {
        Arc::new(Maven)
    }
}

impl Adapter for Maven {
    fn service(&self) -> &'static str {
        SERVICE
    }
    fn ops(&self) -> &'static [Op] {
        OPS
    }
    fn conversation(&self) -> Box<dyn Conversation> {
        Box::new(MavenConversation)
    }
    /// Maven reads the settings below as its global settings. Its wagon transport, because the JDK's (Maven's
    /// default since 3.9) drops a refusal's reason, so Maven would say only "HTTP Status: 403", not why.
    /// sbt reads only the repositories below, for itself and every build.
    fn env(&self, _base: &str, _details: &Value) -> Vec<(String, String)> {
        vec![
            ("MAVEN_ARGS".into(), format!("--global-settings {SETTINGS} -Dmaven.resolver.transport=wagon")),
            ("SBT_OPTS".into(), format!("-Dsbt.override.build.repos=true -Dsbt.repository.config={SBT_REPOSITORIES}")),
        ]
    }
    /// `central` is this registry, its plugin repository under `-plugins`, and every other repository a build names
    /// is mirrored here: nothing comes from anywhere else.
    fn files(&self, base: &str, _details: &Value) -> Vec<(String, Vec<u8>)> {
        let url = base.trim_end_matches('/');
        let settings = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- Carbon's registries: every Maven repository, through this machine's own registry. -->
<settings>
  <mirrors>
    <mirror>
      <id>carbon</id>
      <name>This machine's registry</name>
      <url>{url}</url>
      <mirrorOf>*,!central</mirrorOf>
    </mirror>
  </mirrors>
  <profiles>
    <profile>
      <id>carbon</id>
      <repositories>
        <repository>
          <id>central</id>
          <url>{url}</url>
          <snapshots><enabled>false</enabled></snapshots>
        </repository>
      </repositories>
      <pluginRepositories>
        <pluginRepository>
          <id>central</id>
          <url>{url}/{PLUGINS}</url>
          <snapshots><enabled>false</enabled></snapshots>
        </pluginRepository>
      </pluginRepositories>
    </profile>
  </profiles>
  <activeProfiles>
    <activeProfile>carbon</activeProfile>
  </activeProfiles>
</settings>
"#
        );
        // Every Maven repository a Gradle build declares, pointed here by what it's for: a project's dependencies, or
        // its build's plugins and build logic (`pluginManagement`, `buildscript`), whose Gradle Plugin Portal is the
        // registry's too. The Portal is added to `pluginManagement` before the settings script runs, as Gradle's own
        // default would be.
        let gradle = format!(
            r#"// Carbon's registries: every Maven repository Gradle is given, through this machine's own registry.
def carbonDependencies = '{url}'
def carbonPlugins = '{url}/{PLUGINS}'
def carbonPortal = '{url}/{GRADLE_PLUGINS}'
def carbonPoint = {{ repos, boolean build ->
    repos.configureEach {{ r ->
        if (r instanceof MavenArtifactRepository) {{
            r.url = r.url.toString().contains('plugins.gradle.org') ? carbonPortal : (build ? carbonPlugins : carbonDependencies)
            r.allowInsecureProtocol = true
        }}
    }}
}}
gradle.beforeSettings {{ settings ->
    settings.pluginManagement.repositories.gradlePluginPortal()
    carbonPoint(settings.pluginManagement.repositories, true)
    carbonPoint(settings.buildscript.repositories, true)
    carbonPoint(settings.dependencyResolutionManagement.repositories, false)
}}
gradle.allprojects {{ p ->
    carbonPoint(p.buildscript.repositories, true)
    carbonPoint(p.repositories, false)
}}
"#
        );
        // sbt refuses a plain-HTTP repository unless it says so.
        let sbt = format!("[repositories]\n  local\n  carbon: {url}/, allowInsecureProtocol\n");
        vec![(SETTINGS.into(), settings.into_bytes()), (GRADLE_INIT.into(), gradle.into_bytes()), (SBT_REPOSITORIES.into(), sbt.into_bytes())]
    }
}

struct MavenConversation;

const METADATA: &str = "maven-metadata.xml";

fn parse_path(path: &str) -> Option<Call> {
    let segments: Vec<String> = path.strip_prefix('/')?.split('/').map(decode_segment).collect::<Option<_>>()?;
    let mut s: Vec<&str> = segments.iter().map(String::as_str).collect();
    let prefix = s.first().copied().filter(|p| *p == PLUGINS || *p == GRADLE_PLUGINS);
    if prefix.is_some() {
        s.remove(0);
    }
    let call = parse_folders(&s)?;
    Some(match prefix {
        Some(PLUGINS) => call.with("for", "plugins"),
        Some(_) => call.with("repo", "gradle-plugins"),
        None => call,
    })
}

fn parse_folders(s: &[&str]) -> Option<Call> {
    let (last, folders) = s.split_last()?;
    if let Some(sum) = last.strip_prefix(METADATA) {
        let mut call = Call::new(SERVICE, "metadata").with("path", folders.join("/"));
        match sum {
            "" => {}
            ".sha1" | ".md5" | ".sha256" | ".sha512" => call = call.with("checksum", &sum[1..]),
            _ => return None,
        }
        return Some(call);
    }
    // A version's file: at least one group segment, the artifact, the version, and the file's name, which is theirs.
    let [group @ .., artifact, version] = folders else { return None };
    if group.is_empty() || !last.strip_prefix(&format!("{artifact}-{version}")).is_some_and(is_rest) {
        return None;
    }
    Some(Call::new(SERVICE, "file").with("group", group.join(".")).with("artifact", *artifact).with("version", *version).with("file", *last))
}

impl Conversation for MavenConversation {
    fn parse(&mut self, req: &Request, path: &str) -> Result<Call, Native> {
        if req.method != "GET" && req.method != "HEAD" {
            return Err(Native::plain(405, "Method Not Allowed", "this registry only reads").header("Allow", "GET, HEAD"));
        }
        parse_path(path).ok_or_else(|| Native::plain(404, "Not Found - not something Maven is answered with here", "not something Maven is answered with here"))
    }

    fn reply(&mut self, call: &Call, answer: Answer) -> Native {
        let kind = match (call.op, call.param("checksum"), call.param("file")) {
            ("metadata", None, _) => "application/xml",
            (_, _, Some(f)) if f.ends_with(".pom") => "application/xml",
            ("file", _, Some(f)) if !f.ends_with(".sha1") && !f.ends_with(".md5") && !f.ends_with(".sha256") && !f.ends_with(".sha512") && !f.ends_with(".asc") => "application/octet-stream",
            _ => "text/plain; charset=utf-8",
        };
        match answer {
            Answer::Bytes { reader, size, .. } => Native::new(200, "OK").header("Content-Type", kind).stream(reader, size),
            // Maven shows a failure's status line, not its body, so the reason goes there too.
            Answer::Refused { code, message } => {
                let (status, reason) = match code.as_str() {
                    "not_found" | "invalid" => (404, "Not Found"),
                    "forbidden" | "unauthorized" => (403, "Forbidden"),
                    "too_large" => (413, "Too Large"),
                    _ => (503, "Unavailable"),
                };
                Native::plain(status, &format!("{reason} - {}", clean(&message)), &message)
            }
            Answer::Json(_) => Native::plain(502, "Bad Gateway", &format!("the package server answered maven {} with JSON", call.op)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(path: &str) -> Option<(&'static str, Vec<(&'static str, String)>)> {
        let c = parse_path(path)?;
        c.check(OPS).ok()?;
        Some((c.op, c.params))
    }

    fn p(k: &'static str, v: &str) -> (&'static str, String) {
        (k, v.into())
    }

    #[test]
    fn mavens_paths_are_calls() {
        assert_eq!(call("/org/apache/commons/commons-lang3/maven-metadata.xml"), Some(("metadata", vec![p("path", "org/apache/commons/commons-lang3")])));
        assert_eq!(call("/org/apache/maven/plugins/maven-metadata.xml.sha1"), Some(("metadata", vec![p("path", "org/apache/maven/plugins"), p("checksum", "sha1")])));
        let file = |g: &str, a: &str, v: &str, f: &str| Some(("file", vec![p("group", g), p("artifact", a), p("version", v), p("file", f)]));
        assert_eq!(call("/org/apache/commons/commons-lang3/3.14.0/commons-lang3-3.14.0.jar"), file("org.apache.commons", "commons-lang3", "3.14.0", "commons-lang3-3.14.0.jar"));
        assert_eq!(call("/org/apache/commons/commons-lang3/3.14.0/commons-lang3-3.14.0.pom.sha1"), file("org.apache.commons", "commons-lang3", "3.14.0", "commons-lang3-3.14.0.pom.sha1"));
        assert_eq!(call("/junit/junit/4.13.2/junit-4.13.2-sources.jar"), file("junit", "junit", "4.13.2", "junit-4.13.2-sources.jar"));
        assert_eq!(call("/io/netty/netty-transport-native-epoll/4.1.100.Final/netty-transport-native-epoll-4.1.100.Final-linux-x86_64.jar"), file("io.netty", "netty-transport-native-epoll", "4.1.100.Final", "netty-transport-native-epoll-4.1.100.Final-linux-x86_64.jar"));
        assert_eq!(call("/org/x/dist/1.0/dist-1.0-bin.tar.gz"), file("org.x", "dist", "1.0", "dist-1.0-bin.tar.gz"));
        assert_eq!(call("/org/x/lib/1.0/lib-1.0.module"), file("org.x", "lib", "1.0", "lib-1.0.module"));
        let mut plugin = file("org.apache.maven.plugins", "maven-jar-plugin", "3.4.1", "maven-jar-plugin-3.4.1.jar").unwrap();
        plugin.1.push(p("for", "plugins"));
        assert_eq!(call("/-plugins/org/apache/maven/plugins/maven-jar-plugin/3.4.1/maven-jar-plugin-3.4.1.jar"), Some(plugin));
        assert_eq!(call("/-plugins/org/apache/maven/plugins/maven-metadata.xml"), Some(("metadata", vec![p("path", "org/apache/maven/plugins"), p("for", "plugins")])));
        let mut marker = file("org.example.greet", "org.example.greet.gradle.plugin", "1.0", "org.example.greet.gradle.plugin-1.0.pom").unwrap();
        marker.1.push(p("repo", "gradle-plugins"));
        assert_eq!(call("/-gradle-plugins/org/example/greet/org.example.greet.gradle.plugin/1.0/org.example.greet.gradle.plugin-1.0.pom"), Some(marker));
    }

    #[test]
    fn anything_else_is_refused_before_its_call() {
        for path in [
            "/org/x/lib/1.0/other-1.0.jar",
            "/org/x/lib/1.0/lib-1.0",
            "/org/x/lib/1.0/lib-1.01.jar",
            "/lib/1.0/lib-1.0.jar",
            "/org/x/lib/1.0-SNAPSHOT/lib-1.0-SNAPSHOT.jar",
            "/org/../lib/1.0/lib-1.0.jar",
            "/org/x/lib/1.0/lib-1.0.jar%2F..",
            "/org/x/maven-metadata.xml.txt",
            "/.hidden/maven-metadata.xml",
            "/org/x/lib/1.0/lib-1.0..jar",
            "/-plugins/-plugins/org/x/lib/1.0/lib-1.0.jar",
            "/-gradle-plugins/-plugins/org/x/lib/1.0/lib-1.0.jar",
            "/",
        ] {
            assert!(call(path).is_none(), "{path}");
        }
    }

    #[test]
    fn the_machine_mirrors_everything_here() {
        let files = Maven.files("http://registry.carbon/maven/", &Value::Null);
        assert_eq!(files[0].0, SETTINGS);
        let text = String::from_utf8(files[0].1.clone()).unwrap();
        assert!(text.contains("<url>http://registry.carbon/maven</url>") && text.contains("<url>http://registry.carbon/maven/-plugins</url>"));
        assert!(text.contains("<mirrorOf>*,!central</mirrorOf>") && text.contains("<activeProfile>carbon</activeProfile>"));
        let gradle = String::from_utf8(files[1].1.clone()).unwrap();
        assert_eq!(files[1].0, GRADLE_INIT);
        assert!(gradle.contains("'http://registry.carbon/maven/-gradle-plugins'") && gradle.contains("allowInsecureProtocol = true"));
        assert_eq!((files[2].0.as_str(), String::from_utf8(files[2].1.clone()).unwrap()), (SBT_REPOSITORIES, "[repositories]\n  local\n  carbon: http://registry.carbon/maven/, allowInsecureProtocol\n".to_string()));
        let env = Maven.env("http://registry.carbon/maven/", &Value::Null);
        assert_eq!(env[0], ("MAVEN_ARGS".to_string(), format!("--global-settings {SETTINGS} -Dmaven.resolver.transport=wagon")));
        assert!(env[1].1.contains(&format!("-Dsbt.repository.config={SBT_REPOSITORIES}")));
        assert!(is_version("4.1.100.Final") && is_version("1.0-rc1+build") && !is_version("1.0-SNAPSHOT") && is_group("org.apache") && !is_group("org..apache"));
    }
}
