//! pdh.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "PDH.dll";
    missing PdhAddEnglishCounterA;
    missing PdhCloseQuery;
    missing PdhCollectQueryData;
    missing PdhGetFormattedCounterArrayA;
    missing PdhOpenQueryA;
    // Not written yet: what larger programs import.
    missing PdhAddEnglishCounterW;
    missing PdhGetFormattedCounterValue;
    missing PdhOpenQueryW;
}
