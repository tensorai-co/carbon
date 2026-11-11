# obsidian

**Not started.** A third kernel, for Android programs: native NDK binaries and libraries on bionic's `libc.so`, and
later ART running APKs. The name goes with carbon (volcanic glass), beside lambda (Linux) and atlas (Windows), and is
distinct from Android's own vocabulary (`bionic`, `art`, `dalvik`, `binder`, `ashmem`, `treble`, `apex`). Its crates
would be `obsidian` and, if it needs one, `obsidian-abi`.

## What Android needs from a kernel

Android's userland is Linux's system calls plus a handful of Android devices:

1. **Linux's system calls**, all of them that lambda has: bionic makes the same `svc #0` / `syscall` calls (`mmap`,
   `clone`, `futex`, `epoll`, `io_uring`, `prctl`, sockets, signals) as glibc.
2. **Binder** (`/dev/binder`, `/dev/hwbinder`, `/dev/vndbinder`): Android's IPC, with capabilities as handles and
   transactions through a buffer each process maps (`BINDER_WRITE_READ`). `servicemanager` and every system service
   speak it.
3. **Shared memory** (`/dev/ashmem`, and `memfd`): named regions with pin and unpin (`ASHMEM_SET_NAME`,
   `ASHMEM_SET_SIZE`, `ASHMEM_PIN`, `ASHMEM_UNPIN`), backed by frames from exec's `Memory`, shared between spaces.
4. **System properties** (`/dev/__properties__`): a shared area bionic reads without locks
   (`__system_property_get` for `ro.build.version.sdk`, `ro.product.cpu.abi`, …), made in memory like `/proc`.
5. **Logging** (`/dev/socket/logdw`, `/dev/pmsg0`): `liblog`'s writes (`__android_log_buf_write`, what `logcat`
   reads), routed to Carbon's own tracing.
6. **Graphics**: Android's windows (`ANativeWindow`, `AHardwareBuffer`, SurfaceFlinger and gralloc's roles) as
   top-level windows on [`display`](../../display/README.md)'s desktop, so `--display host` and `--display headless`
   (and computer use by agents) work for Android apps as for the other two kernels.

The rule for every kernel holds: the kernel brings the OS (Binder, properties, the devices, SurfaceFlinger's part);
the image or APK brings the user's software (bionic, the app, its libraries).

## The question to settle first

How it relates to lambda. kernels-core is already the shared part for every kernel; the question is whether obsidian
shares lambda too.

- **A: its own kernel on kernels-core** (`kernels/obsidian`). Built around Android's bionic, Binder, Zygote and APK
  lifecycle, without lambda's desktop-Linux parts. But it would repeat lambda's ~300 system calls, or move most of
  lambda into kernels-core.
- **B: lambda, with Android's devices added.** lambda already runs Linux's system calls on both ISAs; obsidian would
  add `/dev/binder`, `/dev/ashmem`, `/dev/__properties__`, logd's sockets and the graphics path as `os/` parts, and
  pick Android's boot (init, `servicemanager`, Zygote) instead of an image's command.

Since almost all of what Android asks is Linux's ABI, B looks like far less to write. That's a suggestion, not a
decision.

## To do

- [ ] Settle A or B.
- [ ] A static NDK program (bionic, no Java) running, both ISAs.
- [ ] Properties, logging and ashmem; then Binder with `servicemanager`.
- [ ] Windows on the desktop; then ART and an APK.
