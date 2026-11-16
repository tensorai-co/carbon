//! Carbon as a library, as an agent would use it: a command and what it wrote; a machine whose programs, output and
//! connections are decided as they happen; and a machine kept running, `exec`ed into and watched.
//!
//! ```sh
//! cargo build --release -p carbon --example agent
//! codesign --force --sign - --entitlements carbon.entitlements target/release/examples/agent   # macOS
//! target/release/examples/agent              # debian:13, pulled if it isn't here
//! ```

use carbon::hooks::{Decision, Filter, Interest, Reply};
use carbon::{Audio, Carbon, Display, Network, Rules};

fn main() -> carbon::Result<()> {
    carbon::init();
    let carbon = Carbon::open()?;
    let image = std::env::args().nth(1).unwrap_or_else(|| "debian:13".to_string());
    // Each machine here is for one thing: no screen, no sound, removed when it ends.
    let machine = || carbon.machine(&image).display(Display::None).audio(Audio::None).rm(true);

    // 1. A command in a machine of its own, with no network, and what it wrote.
    let out = machine().network(Network::None).memory("512m").output(["sh", "-c", "echo hello from $(uname -sm)"])?;
    print!("1. {}", out.stdout_lossy());

    // 2. Its programs and output decided as they happen: `apt-get` answered without running, `uname` rewritten, a
    //    password redacted from what it writes, `chmod 777` refused by a rule, and the network allowed one host only.
    let out = machine()
        .on_exec(Filter::program("apt-get"), |_| Decision::answer(Reply::new().stdout("curl is already the newest version.\n").exit(0)))
        .on_exec(Filter::program("uname"), |e| if e.args().is_empty() { Decision::rewrite(e.clone().arg("-sr")) } else { Decision::pass() })
        .on_output(Filter::any(), |o| if o.text().contains("hunter2") { o.replace(o.text().replace("hunter2", "*******")) } else { Decision::pass() })
        .rules(Rules::new().deny_exec("chmod 777 *").deny("0.0.0.0/0")?.deny("::/0")?.allow("example.com:80")?)
        .output([
            "bash",
            "-c",
            "apt-get install -y curl; uname; echo password=hunter2; chmod 777 /etc 2>/dev/null; echo chmod=$?; \
             for h in example.com one.one.one.one; do (exec 3<>/dev/tcp/$h/80) 2>/dev/null && echo $h: connected || echo $h: refused; done",
        ])?;
    for line in out.stdout_lossy().lines() {
        println!("2. {line}");
    }

    // 3. A machine kept running: watched, `exec`ed into from here, stopped, and waited for.
    let running = machine().name("agent-demo").start(["sleep", "300"])?;
    let seen = running.hooks().observe(Interest::exec(), 64);
    let release = running.machine().exec(["sh", "-c", "head -1 /etc/os-release"]).output()?;
    print!("3. {} said: {}", running.name(), release.stdout_lossy());
    for observed in std::iter::from_fn(|| seen.try_next()) {
        println!("3. it ran: {} ({})", observed.event.describe(), observed.verdict.name());
    }
    running.stop();
    let name = running.name().to_string();
    println!("3. {name} ended: {}", running.wait()?);
    Ok(())
}
