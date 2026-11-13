//! wevtapi.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "WEVTAPI.dll";
    missing EvtClose;
    missing EvtCreateRenderContext;
    missing EvtNext;
    missing EvtQuery;
    missing EvtRender;
}
