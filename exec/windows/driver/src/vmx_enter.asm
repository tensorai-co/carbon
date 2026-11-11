; Entering and leaving the guest, and the few instructions C can't reach.
;
; WinKvmEnter keeps the Windows x64 ABI whole for its caller: rbx, rbp, rdi, rsi, r12-r15, xmm6-xmm15, MXCSR's and
; the x87's control words, and rflags come back as they were. The guest's general registers go in and out of its
; WINKVM_GUEST_CONTEXT, and its extended state in and out of its XSAVE area, around every entry, so nothing of the
; guest is left in the registers once this returns. ds and es are put back too, as a VM exit leaves them RPL 0.

GC_XSAVE    equ 128
GC_XMASK    equ 136
GC_CR2      equ 144

HOST_RSP    equ 6C14h
HOST_RIP    equ 6C16h

; Saved on the stack: xmm6-xmm15, then MXCSR, the x87 control word, ds and es.
SAVED       equ 0B0h
S_MXCSR     equ 0A0h
S_FPCW      equ 0A4h
S_DS        equ 0A8h
S_ES        equ 0AAh

.code

; UINT64 WinKvmEnter(WINKVM_GUEST_CONTEXT* context, UINT64 launched)
; Returns 0 after a VM exit, 1 if VMLAUNCH or VMRESUME itself failed (VM_INSTRUCTION_ERROR says why).
WinKvmEnter PROC
    push    rbx
    push    rbp
    push    rdi
    push    rsi
    push    r12
    push    r13
    push    r14
    push    r15
    pushfq
    cli
    sub     rsp, SAVED
    movdqu  xmmword ptr [rsp+000h], xmm6
    movdqu  xmmword ptr [rsp+010h], xmm7
    movdqu  xmmword ptr [rsp+020h], xmm8
    movdqu  xmmword ptr [rsp+030h], xmm9
    movdqu  xmmword ptr [rsp+040h], xmm10
    movdqu  xmmword ptr [rsp+050h], xmm11
    movdqu  xmmword ptr [rsp+060h], xmm12
    movdqu  xmmword ptr [rsp+070h], xmm13
    movdqu  xmmword ptr [rsp+080h], xmm14
    movdqu  xmmword ptr [rsp+090h], xmm15
    stmxcsr dword ptr [rsp+S_MXCSR]
    fnstcw  word ptr [rsp+S_FPCW]
    mov     ax, ds
    mov     word ptr [rsp+S_DS], ax
    mov     ax, es
    mov     word ptr [rsp+S_ES], ax

    mov     r8, rcx                     ; the context
    mov     rbx, rdx                    ; launched

    ; The guest's extended state.
    mov     r9, [r8+GC_XSAVE]
    mov     eax, dword ptr [r8+GC_XMASK]
    mov     edx, dword ptr [r8+GC_XMASK+4]
    db      49h, 0Fh, 0AEh, 29h         ; xrstor64 [r9]

    ; The guest's cr2: VMX doesn't switch it.
    mov     rax, [r8+GC_CR2]
    mov     cr2, rax

    ; The context goes on the stack, where the exit finds it.
    push    r8
    mov     rax, HOST_RSP
    vmwrite rax, rsp
    lea     rdx, qword ptr WinKvmExit   ; RIP-relative
    mov     rax, HOST_RIP
    vmwrite rax, rdx

    test    rbx, rbx                    ; the movs below keep the flags
    mov     rax, [r8+000h]
    mov     rcx, [r8+008h]
    mov     rdx, [r8+010h]
    mov     rbx, [r8+018h]
    mov     rbp, [r8+028h]
    mov     rsi, [r8+030h]
    mov     rdi, [r8+038h]
    mov     r9,  [r8+048h]
    mov     r10, [r8+050h]
    mov     r11, [r8+058h]
    mov     r12, [r8+060h]
    mov     r13, [r8+068h]
    mov     r14, [r8+070h]
    mov     r15, [r8+078h]
    mov     r8,  [r8+040h]
    jnz     Resume
    vmlaunch
    jmp     Failed
Resume:
    vmresume

Failed:
    ; Still in the host. The context was never used.
    pop     r8
    mov     r11, 1
    jmp     Back

WinKvmExit:
    ; A VM exit: rsp is where it was at the vmwrite, with the context on top.
    xchg    r8, [rsp]                   ; r8: the context; the stack: the guest's r8
    mov     [r8+000h], rax
    mov     [r8+008h], rcx
    mov     [r8+010h], rdx
    mov     [r8+018h], rbx
    mov     [r8+028h], rbp
    mov     [r8+030h], rsi
    mov     [r8+038h], rdi
    mov     [r8+048h], r9
    mov     [r8+050h], r10
    mov     [r8+058h], r11
    mov     [r8+060h], r12
    mov     [r8+068h], r13
    mov     [r8+070h], r14
    mov     [r8+078h], r15
    pop     rax
    mov     [r8+040h], rax
    mov     rax, cr2
    mov     [r8+GC_CR2], rax

    mov     r9, [r8+GC_XSAVE]
    mov     eax, dword ptr [r8+GC_XMASK]
    mov     edx, dword ptr [r8+GC_XMASK+4]
    db      49h, 0Fh, 0AEh, 21h         ; xsave64 [r9]
    xor     r11, r11

Back:
    mov     ax, word ptr [rsp+S_DS]
    mov     ds, ax
    mov     ax, word ptr [rsp+S_ES]
    mov     es, ax
    fninit
    fldcw   word ptr [rsp+S_FPCW]
    ldmxcsr dword ptr [rsp+S_MXCSR]
    movdqu  xmm6,  xmmword ptr [rsp+000h]
    movdqu  xmm7,  xmmword ptr [rsp+010h]
    movdqu  xmm8,  xmmword ptr [rsp+020h]
    movdqu  xmm9,  xmmword ptr [rsp+030h]
    movdqu  xmm10, xmmword ptr [rsp+040h]
    movdqu  xmm11, xmmword ptr [rsp+050h]
    movdqu  xmm12, xmmword ptr [rsp+060h]
    movdqu  xmm13, xmmword ptr [rsp+070h]
    movdqu  xmm14, xmmword ptr [rsp+080h]
    movdqu  xmm15, xmmword ptr [rsp+090h]
    add     rsp, SAVED
    popfq                               ; interrupts back as they were: a host interrupt that made the exit runs now
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rsi
    pop     rdi
    pop     rbp
    pop     rbx
    mov     rax, r11
    ret
WinKvmEnter ENDP

WinKvmReadCs PROC
    mov     ax, cs
    ret
WinKvmReadCs ENDP

WinKvmReadSs PROC
    mov     ax, ss
    ret
WinKvmReadSs ENDP

WinKvmReadDs PROC
    mov     ax, ds
    ret
WinKvmReadDs ENDP

WinKvmReadEs PROC
    mov     ax, es
    ret
WinKvmReadEs ENDP

WinKvmReadFs PROC
    mov     ax, fs
    ret
WinKvmReadFs ENDP

WinKvmReadGs PROC
    mov     ax, gs
    ret
WinKvmReadGs ENDP

WinKvmReadTr PROC
    str     ax
    ret
WinKvmReadTr ENDP

; VOID WinKvmInvept(UINT64 type, PVOID descriptor)
WinKvmInvept PROC
    db      66h, 0Fh, 38h, 80h, 0Ah     ; invept rcx, oword ptr [rdx]
    ret
WinKvmInvept ENDP

END
