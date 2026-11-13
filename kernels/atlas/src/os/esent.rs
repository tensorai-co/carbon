//! esent.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "ESENT.dll";
    missing JetAttachDatabase2W;
    missing JetBeginSessionW;
    missing JetCloseTable;
    missing JetCreateInstanceW;
    missing JetGetTableColumnInfoW;
    missing JetInit;
    missing JetMove;
    missing JetOpenDatabaseW;
    missing JetOpenTableW;
    missing JetRetrieveColumn;
    missing JetSetSystemParameterW;
    missing JetTerm;
}
