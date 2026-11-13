//! comdlg32.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "COMDLG32.dll";
    missing CommDlgExtendedError;
    missing GetOpenFileNameW;
    missing GetSaveFileNameW;
    // Not written yet: what larger programs import.
    missing PrintDlgExW;
}
