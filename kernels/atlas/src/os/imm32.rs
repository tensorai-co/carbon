//! imm32.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "IMM32.dll";
    missing ImmAssociateContext;
    missing ImmCreateContext;
    missing ImmDestroyContext;
    missing ImmGetCandidateListCountW;
    missing ImmGetCandidateListW;
    missing ImmGetCompositionFontW;
    missing ImmGetCompositionStringW;
    missing ImmGetContext;
    missing ImmGetConversionStatus;
    missing ImmGetIMEFileNameA;
    missing ImmGetOpenStatus;
    missing ImmGetProperty;
    missing ImmNotifyIME;
    missing ImmReleaseContext;
    missing ImmSetCandidateWindow;
    missing ImmSetCompositionStringW;
    missing ImmSetCompositionWindow;
    missing ImmSetConversionStatus;
    missing ImmSetOpenStatus;
    // Not written yet: what larger programs import.
    missing ImmGetIMEFileNameW;
}
