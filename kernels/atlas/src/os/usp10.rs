//! usp10.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "USP10.dll";
    missing ScriptFreeCache;
    missing ScriptItemize;
    missing ScriptPlace;
    missing ScriptShape;
}
