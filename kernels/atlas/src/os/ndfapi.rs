//! ndfapi.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "NDFAPI.dll";
    missing NdfCloseIncident;
    missing NdfCreateWebIncident;
    missing NdfExecuteDiagnosis;
}
