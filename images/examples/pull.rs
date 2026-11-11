//! Pulls an image and lists what's in it: `pull <name> [os/arch]`, into `CARBON_HOME` (or `~/.carbon`). Each file's
//! metadata is kept as lambda keeps it: mode, owner, group, device.

fn main() {
    let mut args = std::env::args().skip(1);
    let name = args.next().unwrap_or_else(|| "debian:13".into());
    let platform = args.next().and_then(|p| images::Platform::parse(&p)).unwrap_or_else(images::Platform::linux_here);
    let images = images::Images::open(None).unwrap_or_else(|e| panic!("{e}"));
    let started = std::time::Instant::now();
    let meta = |e: images::Entry| [e.mode, e.uid, e.gid, e.rdev as u32];
    let image = images.pull(&name, &platform, meta, [0o755, 0, 0, 0], &mut |what| eprintln!("  {what}")).unwrap_or_else(|e| panic!("{e}"));
    println!("{} ({}) in {:.1}s: index {}", image.name, image.platform, started.elapsed().as_secs_f64(), &image.index[..16]);
    println!("config: {:?}", image.config);
    let index = images.index(&image).unwrap_or_else(|e| panic!("{e}"));
    let disk: layers::Fs<[u32; 4]> = layers::Fs::open(Some(&index), Box::new(images.store().unwrap()), [0; 4], false).unwrap();
    let names: Vec<String> = disk.entries(layers::ROOT).unwrap().into_iter().map(|(name, ..)| name).collect();
    println!("/: {}", names.join(" "));
    if let Ok(ino) = disk.resolve(layers::ROOT, "/etc/os-release", true) {
        let text = String::from_utf8_lossy(&disk.contents(ino).unwrap()).into_owned();
        println!("{}", text.lines().next().unwrap_or(""));
    }
    println!("index: {} KiB", index.len() / 1024);
}
