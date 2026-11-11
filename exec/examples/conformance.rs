//! exec's conformance suite: the same tiny programs on both backends of this host, in every ISA each runs, giving the
//! same stops, registers and memory. A backend is done when it passes. On macOS the binary must be signed with
//! `carbon.entitlements` first:
//!
//! ```sh
//! cargo build --release --example conformance
//! codesign --force --sign - --entitlements carbon.entitlements target/release/examples/conformance
//! target/release/examples/conformance            # or: conformance vm, conformance sandbox
//! ```
//!
//! Each program is loaded at a base address, with a data page 64 KiB above it, nothing mapped 512 KiB above it, and a
//! stack at the top of 4 MiB. Programs reach the data page and the hole PC-relatively, so they work at any base. The
//! machine code is assembled from the assembly beside it with LLVM (`clang -c`, then `llvm-objcopy -O binary`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use exec::{Cpu, Exec, Isa, Isolation, Options, PAGE, Prot, Regs, SpaceId, Stop, Tuning, run_if_stub};

const DATA: u64 = 0x1_0000;
const HOLE: u64 = 0x8_0000;
const SIZE: u64 = 0x40_0000;
const STACK: u64 = 0x1_0000;

/// One program, loaded and ready to run.
struct Program {
    // The CPU goes before its exec: hvf can't destroy a VM that still has a vCPU.
    cpu: Cpu,
    exec: Exec,
    base: u64,
    space: SpaceId,
}

impl Program {
    fn load(isolation: Isolation, isa: Isa, code: &[u8], data: &[u8]) -> Result<Program, String> {
        Self::load_as(isolation, isa, code, data, Prot::RX, Tuning::default())
    }

    /// With the code mapped `code_prot`, and exec tuned as `tuning` says.
    fn load_as(isolation: Isolation, isa: Isa, code: &[u8], data: &[u8], code_prot: Prot, tuning: Tuning) -> Result<Program, String> {
        // x86 translated on Apple silicon wants the CPU's own x86 memory ordering.
        let tso = isa.is_x86() && Isa::host() == Isa::Arm64;
        let options = Options { memory: 64 << 20, isolation, tso, tuning };
        let (mut exec, mut cpu) = Exec::open(options).map_err(|e| e.to_string())?;
        let (base, space) = {
            let mut turn = exec.enter(&mut cpu);
            let space = turn.new_space(isa).map_err(|e| format!("{e:?}"))?;
            turn.use_space(space);
            // Preemption only when the test asks for it.
            turn.time_slices(false);
            // Somewhere the layout allows: low down, unless the backend keeps it.
            let layout = turn.space().layout();
            let free = |at: u64| at + SIZE <= layout.top && !layout.kept.iter().any(|r| r.start < at + SIZE && at < r.end);
            let base = [0x40_0000u64, 0x10_0000_0000, 0x1000_0000_0000].into_iter().find(|at| free(*at)).ok_or("no room for the program")?;
            let code_end = base + (code.len() as u64).next_multiple_of(PAGE);
            let mut s = turn.space();
            s.map(base..code_end, code_prot).with(code).map_err(|e| format!("{e:?}"))?;
            s.map(base + DATA..base + DATA + PAGE, Prot::RW).with(data).map_err(|e| format!("{e:?}"))?;
            s.map(base + SIZE - STACK..base + SIZE, Prot::RW).zeroed().map_err(|e| format!("{e:?}"))?;
            turn.thread().start(base, base + SIZE);
            (base, space)
        };
        Ok(Program { cpu, exec, base, space })
    }

    /// Runs until it stops for its kernel (preemption only if `preempted`).
    fn run(&mut self, preempted: bool) -> Stop {
        loop {
            let exit = self.cpu.run();
            let mut turn = self.exec.enter(&mut self.cpu);
            match turn.stop(exit) {
                None => {}
                Some(Stop::Preempted) if !preempted => {}
                Some(stop) => return stop,
            }
        }
    }

    fn data(&mut self, len: usize) -> Vec<u8> {
        let mut buffer = vec![0; len];
        let at = self.base + DATA;
        let _ = self.exec.enter(&mut self.cpu).space().read(at, &mut buffer);
        buffer
    }
}

/// What a case expects: the stop's kind and what it carries, at `base`-relative addresses.
fn exits(stop: &Stop, nr: u64, code: u64) -> Result<(), String> {
    match stop {
        Stop::Syscall { nr: n, args } if *n == nr && args[0] == code => Ok(()),
        other => Err(format!("wanted exit({code}) by syscall {nr}, got {other:x?}")),
    }
}

/// The system call that ends the program, in each ISA's Linux numbering.
fn exit_nr(isa: Isa) -> u64 {
    if isa == Isa::Arm64 { 93 } else { 60 }
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex")).collect()
}

/// How a case's program reaches its kernel.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// It doesn't, or only by faulting.
    Neither,
    /// By the ISA's own trap: `syscall`, `svc`. A backend with a gate can't take those.
    Trap,
    /// By calling the backend's gate, whose address is in the data page.
    Gate,
}

/// A case: its name, how it reaches its kernel, its code per ISA (x86-64, ARM64; empty where it has none), and how
/// it's checked.
struct Case {
    name: &'static str,
    reach: Reach,
    x86: &'static str,
    arm64: &'static str,
    check: fn(Isa, Isolation, &[u8]) -> Result<(), String>,
}

/// Loads `code` with empty data and runs it to its first stop.
fn first_stop(isa: Isa, isolation: Isolation, code: &[u8]) -> Result<(Program, Stop), String> {
    let mut p = Program::load(isolation, isa, code, &[])?;
    let stop = p.run(false);
    Ok((p, stop))
}

/// Loads `code` with the backend's gate address in its data page, and runs it to its first stop.
fn first_gate_stop(isa: Isa, isolation: Isolation, code: &[u8]) -> Result<(Program, Stop), String> {
    let gate = gate_of(isolation, isa).ok_or("skip: no gate here")?;
    let mut p = Program::load(isolation, isa, code, &gate.to_le_bytes())?;
    let stop = p.run(false);
    Ok((p, stop))
}

/// Where a backend's programs call to reach their kernel, if it's a gate rather than a trap.
fn gate_of(isolation: Isolation, isa: Isa) -> Option<u64> {
    let (mut exec, mut cpu) = Exec::open(Options { memory: 16 << 20, isolation, ..Options::default() }).ok()?;
    let gate = {
        let mut turn = exec.enter(&mut cpu);
        let space = turn.new_space(isa).ok()?;
        turn.use_space(space);
        turn.space().layout().gate
    };
    drop(cpu);
    gate
}

fn expect_at(stop: &Stop, want: &str, matches: bool) -> Result<(), String> {
    if matches { Ok(()) } else { Err(format!("wanted {want}, got {stop:x?}")) }
}

/// Many blocks, run through several times: `mov ecx, passes; xor eax, eax`, then each pass calls a `ret` and goes
/// through `blocks` blocks of `add eax, i; jmp` to the next, before `dec ecx; jnz`, and exits with eax.
fn many_blocks(blocks: u32, passes: u32) -> (Vec<u8>, u64) {
    let mut code = vec![0xb9];
    code.extend(passes.to_le_bytes());
    code.extend([0x31, 0xc0]);
    let top = code.len();
    code.push(0xe8);
    code.extend([0; 4]); // call sub, patched below
    for i in 1..=blocks {
        code.push(0x05);
        code.extend(i.to_le_bytes());
        code.extend([0xeb, 0x00]);
    }
    code.extend([0xff, 0xc9, 0x0f, 0x85]);
    let back = top as i64 - (code.len() as i64 + 4);
    code.extend((back as i32).to_le_bytes());
    code.extend([0x89, 0xc7, 0xb8, 0x3c, 0, 0, 0, 0x0f, 0x05]);
    let sub = code.len();
    code.push(0xc3);
    let rel = (sub as i64 - (top as i64 + 5)) as i32;
    code[top + 1..top + 5].copy_from_slice(&rel.to_le_bytes());
    let sum = u64::from(blocks) * u64::from(blocks + 1) / 2 * u64::from(passes);
    (code, sum & 0xFFFF_FFFF)
}

/// Runs a program that calls a function returning 1, stops at a system call, then calls it again and exits with the
/// sum, after `change` has done something to the function's code at the stop. 3 when the change was seen, else 2.
fn code_changed_at_a_stop(isa: Isa, iso: Isolation, code: &[u8], change: fn(&mut Program, &[u8]) -> Result<(), String>) -> Result<(), String> {
    let mut p = Program::load(iso, isa, code, &[])?;
    let stop = p.run(false);
    expect_at(&stop, "syscall 1", matches!(stop, Stop::Syscall { nr: 1, .. }))?;
    change(&mut p, code)?;
    let stop = p.run(false);
    exits(&stop, exit_nr(isa), 3)
}

const CASES: &[Case] = &[
    // mov edi, 7; mov eax, 60; syscall  |  mov x0, #7; mov x8, #93; svc #0
    Case { name: "exit(7)", reach: Reach::Trap, x86: "bf07000000b83c0000000f05", arm64: "e00080d2a80b80d2010000d4", check: |isa, iso, code| {
        let (_, stop) = first_stop(isa, iso, code)?;
        exits(&stop, exit_nr(isa), 7)
    } },
    // six arguments, 1 to 6, to system call 99
    Case {
        name: "a system call's six arguments", reach: Reach::Trap,
        x86: "bf01000000be02000000ba0300000041ba0400000041b80500000041b906000000b8630000000f05",
        arm64: "200080d2410080d2620080d2830080d2a40080d2c50080d2680c80d2010000d4",
        check: |isa, iso, code| {
            let (_, stop) = first_stop(isa, iso, code)?;
            expect_at(&stop, "syscall 99 with 1 to 6", matches!(stop, Stop::Syscall { nr: 99, args: [1, 2, 3, 4, 5, 6] }))
        },
    },
    // syscall 1, which the kernel answers 41; then exit(answer + 1)
    Case {
        name: "a system call's answer, and carrying on", reach: Reach::Trap,
        x86: "b8010000000f058d7801b83c0000000f05",
        arm64: "280080d2010000d400040091a80b80d2010000d4",
        check: |isa, iso, code| {
            let (mut p, stop) = first_stop(isa, iso, code)?;
            expect_at(&stop, "syscall 1", matches!(stop, Stop::Syscall { nr: 1, .. }))?;
            p.exec.enter(&mut p.cpu).thread().set_return(41);
            exits(&p.run(false), exit_nr(isa), 42)
        },
    },
    // xor eax, eax; mov ecx, 10; l: add eax, ecx; dec ecx; jnz l; exit(eax)  |  the same, through the stack
    Case {
        name: "a loop, then exit(55)", reach: Reach::Trap,
        x86: "31c0b90a00000001c8ffc975fa89c7b83c0000000f05",
        arm64: "000080d2410180d20000018b210400f1c1ffff54e00f1ff8e20741f8e00302aaa80b80d2010000d4",
        check: |isa, iso, code| {
            let (_, stop) = first_stop(isa, iso, code)?;
            exits(&stop, exit_nr(isa), 55)
        },
    },
    // mov dword [data], 0x1234; exit(0): the kernel reads it back
    Case {
        name: "a store the kernel reads", reach: Reach::Trap,
        x86: "c705f6ff00003412000031ffb83c0000000f05",
        arm64: "0100081080468252200000b9000080d2a80b80d2010000d4",
        check: |isa, iso, code| {
            let (mut p, stop) = first_stop(isa, iso, code)?;
            exits(&stop, exit_nr(isa), 0)?;
            let got = p.data(4);
            if got == 0x1234u32.to_le_bytes() { Ok(()) } else { Err(format!("data is {got:x?}")) }
        },
    },
    // exit(dword [data]), with data 5, then a fork whose copy writes 9: each sees its own
    Case {
        name: "fork, copy-on-write", reach: Reach::Trap,
        x86: "8b3dfaff0000b83c0000000f05",
        arm64: "01000810200040b9a80b80d2010000d4",
        check: |isa, iso, code| {
            let mut p = Program::load(iso, isa, code, &5u32.to_le_bytes())?;
            let (base, parent) = (p.base, p.space);
            let child = {
                let mut turn = p.exec.enter(&mut p.cpu);
                let child = turn.fork_space().map_err(|e| format!("fork: {e:?}"))?;
                turn.use_space(child);
                // A shared page is the kernel's to copy before writing, as a fault would.
                turn.space().make_writable(base + DATA, Prot::RW).map_err(|e| format!("make_writable: {e:?}"))?;
                turn.space().write(base + DATA, &9u32.to_le_bytes()).map_err(|e| format!("write: {e:?}"))?;
                turn.use_space(parent);
                child
            };
            exits(&p.run(false), exit_nr(isa), 5).map_err(|e| format!("parent: {e}"))?;
            let still = p.data(4);
            if still != 5u32.to_le_bytes() {
                return Err(format!("the parent's data became {still:x?}"));
            }
            {
                let mut turn = p.exec.enter(&mut p.cpu);
                turn.use_space(child);
                turn.thread().start(base, base + SIZE);
            }
            exits(&p.run(false), exit_nr(isa), 9).map_err(|e| format!("child: {e}"))
        },
    },
    // nop; here: mov byte [here], 1  |  nop; here: adr x1, here; str w0, [x1]: writing its own code
    Case {
        name: "writing a read-only page", reach: Reach::Neither,
        x86: "90c605f9ffffff010f0b",
        arm64: "1f2003d501000010200000b900000000",
        check: |isa, iso, code| {
            let (p, stop) = first_stop(isa, iso, code)?;
            let (addr, pc) = if isa == Isa::Arm64 { (p.base + 4, p.base + 8) } else { (p.base + 1, p.base + 1) };
            let ok = matches!(stop, Stop::Fault { addr: a, access, pc: at } if a == addr && access.contains(Prot::W) && at == pc);
            expect_at(&stop, &format!("a write fault at {addr:#x}, pc {pc:#x}"), ok)
        },
    },
    // nop; read dword [hole]: nothing's mapped there
    Case {
        name: "reading an unmapped page", reach: Reach::Neither,
        x86: "908b05f9ff07000f0b",
        arm64: "1f2003d5e1ff3f10200040b900000000",
        check: |isa, iso, code| {
            let (p, stop) = first_stop(isa, iso, code)?;
            let (addr, pc) = (p.base + HOLE, if isa == Isa::Arm64 { p.base + 8 } else { p.base + 1 });
            let ok = matches!(stop, Stop::Fault { addr: a, access, pc: at } if a == addr && !access.contains(Prot::W) && at == pc);
            expect_at(&stop, &format!("a read fault at {addr:#x}, pc {pc:#x}"), ok)
        },
    },
    // mov dword [hole], 7; exit(dword [hole]): the write faults, the kernel maps the page, and the program carries on
    Case {
        name: "a fault the kernel answers, and carrying on", reach: Reach::Trap,
        x86: "c705f6ff0700070000008b3df0ff0700b83c0000000f05",
        arm64: "01004010e0008052200000b9200040b9a80b80d2010000d4",
        check: |isa, iso, code| {
            let (mut p, stop) = first_stop(isa, iso, code)?;
            let hole = p.base + HOLE;
            expect_at(&stop, &format!("a write fault at {hole:#x}"), matches!(stop, Stop::Fault { addr, .. } if addr == hole))?;
            p.exec.enter(&mut p.cpu).space().map(hole..hole + PAGE, Prot::RW).zeroed().map_err(|e| format!("{e:?}"))?;
            exits(&p.run(false), exit_nr(isa), 7)
        },
    },
    // mov eax, 60; syscall  |  mov x8, #93; svc #0, on a second CPU given the first's registers with the argument
    // changed: what a fork's child does
    Case {
        name: "a second CPU takes a program's registers", reach: Reach::Trap,
        x86: "b83c0000000f05",
        arm64: "a80b80d2010000d4",
        check: |isa, iso, code| {
            let mut p = Program::load(iso, isa, code, &[])?;
            let mut regs = p.exec.enter(&mut p.cpu).thread().regs();
            match &mut regs {
                Regs::X86(r) => r.gpr[exec::isa::x86::RDI] = 9,
                Regs::Arm64(r) => r.x[0] = 9,
            }
            // A CPU belongs to the thread that adds it (hvf allows one per thread), as a kernel's threads do.
            let (exec, space) = (&mut p.exec, p.space);
            let stop = std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        let mut second = match exec.add_cpu() {
                            Ok(cpu) => cpu,
                            Err(exec::Error::NoMoreCpus) => return Err("skip: one CPU".to_string()),
                            Err(e) => return Err(format!("add_cpu: {e:?}")),
                        };
                        {
                            let mut turn = exec.enter(&mut second);
                            turn.use_space(space);
                            turn.time_slices(false);
                            turn.thread().set_regs(&regs);
                        }
                        loop {
                            let exit = second.run();
                            if let Some(stop) = exec.enter(&mut second).stop(exit).filter(|s| *s != Stop::Preempted) {
                                return Ok(stop);
                            }
                        }
                    })
                    .join()
                    .map_err(|_| "the second CPU's thread panicked".to_string())?
            })?;
            exits(&stop, exit_nr(isa), 9)
        },
    },
    // mov eax, 60; syscall  |  mov x8, #93; svc #0, on more CPUs than the backend has at once, each made on a thread of
    // its own and dropped before the next: what a machine's threads and processes do over its life
    Case {
        name: "CPUs made again in the slots of ones dropped", reach: Reach::Trap,
        x86: "b83c0000000f05",
        arm64: "a80b80d2010000d4",
        check: |isa, iso, code| {
            let mut p = Program::load(iso, isa, code, &[])?;
            let regs = p.exec.enter(&mut p.cpu).thread().regs();
            let rounds = p.exec.caps().max_cpus + 16;
            let (exec, space) = (&mut p.exec, p.space);
            for round in 0..rounds {
                let mut regs = regs;
                match &mut regs {
                    Regs::X86(r) => r.gpr[exec::isa::x86::RDI] = round as u64,
                    Regs::Arm64(r) => r.x[0] = round as u64,
                }
                let stop = std::thread::scope(|scope| {
                    scope
                        .spawn(|| {
                            let mut cpu = match exec.add_cpu() {
                                Ok(cpu) => cpu,
                                Err(exec::Error::NoMoreCpus) if round == 0 => return Err("skip: one CPU".to_string()),
                                Err(e) => return Err(format!("add_cpu, CPU {round} of {rounds}: {e:?}")),
                            };
                            {
                                let mut turn = exec.enter(&mut cpu);
                                turn.use_space(space);
                                turn.time_slices(false);
                                turn.thread().set_regs(&regs);
                            }
                            loop {
                                let exit = cpu.run();
                                if let Some(stop) = exec.enter(&mut cpu).stop(exit).filter(|s| *s != Stop::Preempted) {
                                    return Ok(stop);
                                }
                            }
                        })
                        .join()
                        .map_err(|_| format!("CPU {round}'s thread panicked"))?
                })?;
                exits(&stop, exit_nr(isa), round as u64).map_err(|e| format!("CPU {round} of {rounds}: {e}"))?;
            }
            Ok(())
        },
    },
    // mov ebx, 7; mov eax, 60; call [gate]: on a backend whose programs reach their kernel through a gate
    Case {
        name: "exit(7) through the gate",
        reach: Reach::Gate,
        x86: "bb07000000b83c000000ff15f0ff00000f0b",
        arm64: "",
        check: |isa, iso, code| {
            let (_, stop) = first_gate_stop(isa, iso, code)?;
            exits(&stop, exit_nr(isa), 7)
        },
    },
    // call [gate] with rax 1, answered 41; then exit(answer + 1) through it again
    Case {
        name: "the gate's answer, and carrying on",
        reach: Reach::Gate,
        x86: "b801000000ff15f5ff00008d5801b83c000000ff15e7ff00000f0b",
        arm64: "",
        check: |isa, iso, code| {
            let (mut p, stop) = first_gate_stop(isa, iso, code)?;
            expect_at(&stop, "a call to the gate, number 1", matches!(stop, Stop::Syscall { nr: 1, .. }))?;
            p.exec.enter(&mut p.cpu).thread().set_return(41);
            exits(&p.run(false), exit_nr(isa), 42)
        },
    },
    // nop; nop; int3  |  nop; nop; brk #0: the breakpoint's own address
    Case { name: "a breakpoint", reach: Reach::Neither, x86: "9090cc0f0b", arm64: "1f2003d51f2003d5000020d400000000", check: |isa, iso, code| {
        let (p, stop) = first_stop(isa, iso, code)?;
        let pc = p.base + if isa == Isa::Arm64 { 8 } else { 2 };
        expect_at(&stop, &format!("a breakpoint at {pc:#x}"), matches!(stop, Stop::Breakpoint { pc: at } if at == pc))
    } },
    // nop; nop; ud2  |  nop; nop; udf #0
    Case { name: "an undefined instruction", reach: Reach::Neither, x86: "90900f0b", arm64: "1f2003d51f2003d500000000", check: |isa, iso, code| {
        let (p, stop) = first_stop(isa, iso, code)?;
        let pc = p.base + if isa == Isa::Arm64 { 8 } else { 2 };
        expect_at(&stop, &format!("an undefined instruction at {pc:#x}"), matches!(stop, Stop::Undefined { pc: at } if at == pc))
    } },
    // nop; xor ecx, ecx; mov eax, 1; xor edx, edx; div ecx (ARM64's divide by zero gives 0 and doesn't trap)
    Case { name: "divide by zero", reach: Reach::Neither, x86: "9031c9b80100000031d2f7f10f0b", arm64: "", check: |isa, iso, code| {
        let (p, stop) = first_stop(isa, iso, code)?;
        let pc = p.base + 10;
        expect_at(&stop, &format!("a divide error at {pc:#x}"), matches!(stop, Stop::DivideError { pc: at } if at == pc))
    } },
    // l: jmp l  |  l: b l, preempted from another thread
    Case { name: "preempted from another thread", reach: Reach::Neither, x86: "ebfe", arm64: "00000014", check: |isa, iso, code| {
        let mut p = Program::load(iso, isa, code, &[])?;
        let handle = p.cpu.handle();
        let done = Arc::new(AtomicBool::new(false));
        let stop_asking = done.clone();
        let asker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            while !stop_asking.load(Ordering::Relaxed) {
                handle.preempt();
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let stop = p.run(true);
        done.store(true, Ordering::Relaxed);
        let _ = asker.join();
        expect_at(&stop, "preempted", matches!(stop, Stop::Preempted))
    } },
    // call f; exit(eax). f: push rbx; mov ebx, 40; mov [rsp - 16], rbx; mov rcx, [rsp - 16]; lea eax, [rcx + 2];
    // cpuid; mov eax, 42; pop rbx; ret
    Case {
        name: "call, the stack, cpuid, ret, then exit(42)", reach: Reach::Trap,
        x86: "e80900000089c7b83c0000000f0553bb28000000_48895c24f0488b4c24f08d41020fa2b82a0000005bc3",
        arm64: "",
        check: |isa, iso, code| {
            let (_, stop) = first_stop(isa, iso, code)?;
            exits(&stop, exit_nr(isa), 42)
        },
    },
    // adc and sbb through the carry flag: 15. mov rax, -1; add rax, 1; mov edx, 5; adc rdx, 0; mov eax, 3;
    // cmp eax, 4; mov esi, 10; sbb esi, 0; sbb rcx, rcx; lea edi, [rdx + rsi]; add edi, ecx; exit(edi)
    Case {
        name: "adc and sbb, then exit(15)", reach: Reach::Trap,
        x86: "48c7c0ffffffff4883c001ba050000004883d200b80300000083f804be0a00000083de004819c98d3c3201cfb83c0000000f05",
        arm64: "",
        check: |isa, iso, code| {
            let (_, stop) = first_stop(isa, iso, code)?;
            exits(&stop, exit_nr(isa), 15)
        },
    },
    // adc and sbb on bytes and words: 128 + 1 + 15 + 15 = 159. mov al, 0xff; add al, 1; mov bl, 0x7f; adc bl, 0;
    // seto cl; xor edx, edx; cmp dx, 1; sbb dx, 0; sbb al, 0; and edx, 15; and eax, 15; movzx edi, bl;
    // add edi, edx; add edi, eax; movzx ecx, cl; add edi, ecx; exit(edi)
    Case {
        name: "adc and sbb on bytes and words, then exit(159)", reach: Reach::Trap,
        x86: "b0ff0401b37f80d3000f90c131d26683fa016683da001c0083e20f83e00f0fb6fb01d701c70fb6c901cfb83c0000000f05",
        arm64: "",
        check: |isa, iso, code| {
            let (_, stop) = first_stop(isa, iso, code)?;
            exits(&stop, exit_nr(isa), 159)
        },
    },
    // shld and shrd: rax = 0x123456789abcdef0 shld 8 from rdx = 0xfedcba9876543210; then rdx shrd 12 (by cl) from
    // rax. exit(al + (rdx >> 56)) = 0xfe + 0x0f = 269
    Case {
        name: "shld and shrd, then exit(269)",
        reach: Reach::Trap,
        x86: "48b8f0debc9a7856341248ba1032547698badcfe480fa4d008b90c000000480fadc20fb6f848c1ea384801d7b83c0000000f05",
        arm64: "",
        check: |isa, iso, code| {
            let (_, stop) = first_stop(isa, iso, code)?;
            exits(&stop, exit_nr(isa), 269)
        },
    },
    // A 128-bit dividend: rdx:rax = 0x123456789abcdef0_0fedcba987654321, div rcx = 0xfedcba9876543211. exit(42)
    // when rax is 0x1249249249249237 and rdx 0xe900299c335ccf7a, else exit(1).
    Case {
        name: "a 128-bit div, then exit(42)",
        reach: Reach::Trap,
        x86: "48b821436587a9cbed0f48baf0debc9a7856341248b91132547698badcfe48f7f149b837922449922449124c39c0751649b87acf5c339c2900e94c39c27507bf2a000000eb05bf01000000b83c0000000f05",
        arm64: "",
        check: |isa, iso, code| {
            let (_, stop) = first_stop(isa, iso, code)?;
            exits(&stop, exit_nr(isa), 42)
        },
    },
    // mov ecx, 2; mov edx, 2; mov eax, 1; div rcx: a quotient past 64 bits is a divide error
    Case { name: "a 128-bit div that overflows", reach: Reach::Neither, x86: "b902000000ba02000000b80100000048f7f10f0b", arm64: "", check: |isa, iso, code| {
        let (p, stop) = first_stop(isa, iso, code)?;
        let pc = p.base + 15;
        expect_at(&stop, &format!("a divide error at {pc:#x}"), matches!(stop, Stop::DivideError { pc: at } if at == pc))
    } },
    // Code the program rewrites after running it, on a page it may write: call b (mov eax, 1; ret); mov ebx, eax;
    // mov dword [b + 1], 2; call b; add ebx, eax; exit(ebx). 3 if the second call ran the new code.
    Case {
        name: "code the program rewrites after running it",
        reach: Reach::Trap,
        x86: "e81c00000089c3c7051100000002000000e80b00000001c389dfb83c0000000f05b801000000c3",
        arm64: "",
        check: |isa, iso, code| {
            let mut p = Program::load_as(iso, isa, code, &[], Prot::RWX, Tuning::default())?;
            let stop = p.run(false);
            exits(&stop, exit_nr(isa), 3)
        },
    },
    // Code the kernel writes over after it ran: call b (mov eax, 1; ret); mov ebx, eax; syscall 1, where the kernel
    // makes b return 2; call b; add ebx, eax; exit(ebx).
    Case {
        name: "code the kernel writes over after it ran",
        reach: Reach::Trap,
        x86: "e81900000089c3b8010000000f05e80b00000001c389dfb83c0000000f05b801000000c3",
        arm64: "",
        check: |isa, iso, code| {
            code_changed_at_a_stop(isa, iso, code, |p, _| {
                let at = p.base + 0x1f;
                p.exec.enter(&mut p.cpu).space().write(at, &2u32.to_le_bytes()).map_err(|e| format!("{e:?}"))
            })
        },
    },
    // The same program, with its code page unmapped and mapped again at the stop, now with b returning 2.
    Case {
        name: "code the kernel maps anew where code ran",
        reach: Reach::Trap,
        x86: "e81900000089c3b8010000000f05e80b00000001c389dfb83c0000000f05b801000000c3",
        arm64: "",
        check: |isa, iso, code| {
            code_changed_at_a_stop(isa, iso, code, |p, code| {
                let mut new = code.to_vec();
                new[0x1f] = 2;
                let base = p.base;
                let mut turn = p.exec.enter(&mut p.cpu);
                let mut s = turn.space();
                s.unmap(base..base + PAGE);
                s.map(base..base + PAGE, Prot::RX).with(&new).map_err(|e| format!("{e:?}"))
            })
        },
    },
    // More translated code than a small code cache holds: 3,000 blocks, eight times over, with the cache moving to its
    // other half each time round. exit(the sum).
    Case {
        name: "more code than the code cache holds",
        reach: Reach::Trap,
        x86: "90",
        arm64: "",
        check: |isa, iso, _| {
            let (code, sum) = many_blocks(3000, 8);
            let tuning = Tuning { code_cache: 64 << 10, ..Tuning::default() };
            let mut p = Program::load_as(iso, isa, &code, &[], Prot::RX, tuning)?;
            let stop = p.run(false);
            exits(&stop, exit_nr(isa), sum)
        },
    },
    // bt, bts, btr, btc: 31. mov eax, 10; xor ecx, ecx; bts rax, 0; bt rax, 3; adc rcx, 0; mov edx, 6;
    // btr rax, rdx; btc rax, 2; shl rcx, 4; lea rdi, [rax + rcx]; exit(edi)
    Case {
        name: "bt, bts, btr and btc, then exit(31)", reach: Reach::Trap,
        x86: "b80a00000031c9480fbae800480fbae0034883d100ba06000000480fb3d0480fbaf80248c1e104488d3c08b83c0000000f05",
        arm64: "",
        check: |isa, iso, code| {
            let (_, stop) = first_stop(isa, iso, code)?;
            exits(&stop, exit_nr(isa), 31)
        },
    },
];

fn main() {
    // On macOS the sandbox's stub is this binary again.
    run_if_stub();
    let only = std::env::args().nth(1);
    // A case that hangs fails the suite rather than stopping it forever.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(120));
        println!("FAILED  the suite took over two minutes: a case hung");
        std::process::exit(1);
    });
    let (mut passed, mut failed, mut skipped) = (0, 0, 0);
    for isolation in [Isolation::Vm, Isolation::Sandbox] {
        let wanted = match only.as_deref() {
            Some("vm") => isolation == Isolation::Vm,
            Some("sandbox") => isolation == Isolation::Sandbox,
            _ => true,
        };
        if !wanted {
            continue;
        }
        // What this backend is and runs; hvf allows one VM per process, so it's closed before the cases.
        let (backend, isas) = match Exec::open(Options { memory: 16 << 20, isolation, ..Options::default() }) {
            Ok((exec, _cpu)) => (exec.backend().to_string(), exec.caps().isas),
            Err(why) => {
                println!("skip    {isolation:?}: {why}");
                skipped += 1;
                continue;
            }
        };
        println!("{backend}");
        for isa in [Isa::X86_64, Isa::Arm64] {
            let gate = isas.contains(&isa) && gate_of(isolation, isa).is_some();
            for case in CASES {
                let code = if isa == Isa::Arm64 { case.arm64 } else { case.x86 };
                if code.is_empty() {
                    continue;
                }
                let label = format!("{:<7} {}", if isa == Isa::Arm64 { "arm64" } else { "x86-64" }, case.name);
                if !isas.contains(&isa) {
                    println!("skip    {label}: {backend} doesn't run it");
                    skipped += 1;
                    continue;
                }
                match (case.reach, gate) {
                    (Reach::Trap, true) => {
                        println!("skip    {label}: {backend}'s programs reach their kernel through its gate");
                        skipped += 1;
                        continue;
                    }
                    (Reach::Gate, false) => continue,
                    _ => {}
                }
                let code = hex(&code.replace('_', ""));
                match (case.check)(isa, isolation, &code) {
                    Ok(()) => {
                        println!("ok      {label}");
                        passed += 1;
                    }
                    Err(why) if why.starts_with("skip: ") => {
                        println!("skip    {label}: {}", &why[6..]);
                        skipped += 1;
                    }
                    Err(why) => {
                        println!("FAILED  {label}: {why}");
                        failed += 1;
                    }
                }
            }
        }
    }
    println!("{passed} passed, {failed} failed, {skipped} skipped");
    std::process::exit(failed);
}
