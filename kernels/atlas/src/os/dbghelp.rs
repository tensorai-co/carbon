//! dbghelp.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "DBGHELP.dll";
    missing MiniDumpWriteDump;
    missing StackWalk64;
    missing SymCleanup;
    missing SymFromAddr;
    missing SymFunctionTableAccess64;
    missing SymGetLineFromAddr64;
    missing SymGetModuleBase64;
    missing SymGetSearchPathW;
    missing SymInitialize;
    missing SymSetOptions;
    missing SymSetSearchPathW;
}
