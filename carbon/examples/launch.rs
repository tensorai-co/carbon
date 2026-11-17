//! A machine launched in a process of its own, with rules (decided in its worker) and hooks with code (decided here,
//! over its hook link): `cargo run --release -p carbon --example launch [IMAGE]`, signed on macOS as `carbon` is.
//!
//! With `leave` after the image, it launches the machine and ends, leaving it running with no one to ask: then
//! `carbon exec hooked whoami` is refused, since a hook with code wanted it and its launcher is gone.

use carbon::hooks::{Decision, Filter, Reply};
use carbon::{Carbon, Rules};

fn main() -> carbon::Result<()> {
    carbon::init();
    let carbon = Carbon::open()?;
    let mut args = std::env::args().skip(1);
    let image = args.next().unwrap_or_else(|| "debian".into());
    let leave = args.next().as_deref() == Some("leave");

    let machine = carbon
        .machine(&image)
        .name("hooked")
        .rm(!leave)
        .rules(Rules::new().deny_exec("rm -rf *"))
        .on_exec(Filter::program("whoami"), |_| Decision::answer(Reply::new().stdout("agent\n").exit(0)))
        .on_exec(Filter::program("uname"), |e| Decision::rewrite(e.clone().with_argv(["echo", "rewritten"])))
        .launch(["sleep", "infinity"])?;
    println!("launched {} in process {:?}", machine.name(), machine.info()?.pid);
    if leave {
        return Ok(());
    }

    let run = |argv: &[&str]| -> carbon::Result<String> {
        let out = machine.exec(argv.iter().copied()).output()?;
        Ok(format!("exit {} {}{}", out.code(), out.stdout_lossy().trim(), out.stderr_lossy().trim()))
    };
    println!("whoami (answered here): {}", run(&["whoami"])?);
    println!("uname (rewritten here): {}", run(&["uname", "-a"])?);
    println!("rm -rf (a rule, in the worker): {}", run(&["sh", "-c", "mkdir -p /tmp/x && rm -rf /tmp/x; echo $?"])?);
    println!("ls (nothing decides it): {}", run(&["ls", "/"])?.lines().next().unwrap_or_default());
    machine.stop()?;
    machine.wait(Some(std::time::Duration::from_secs(15)));
    println!("stopped: exit {:?}", machine.info().map(|i| i.exit).unwrap_or(None));
    Ok(())
}
