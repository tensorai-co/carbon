//! oleaut32.dll: BSTRs and VARIANTs, guest code (`atlas-guest`'s `oleaut`), at Windows' own ordinals, which programs
//! import them by.

use super::dll;

dll! {
    "OLEAUT32.dll" @ 2;
    guest SysAllocString => AtlasSysAllocString;
    guest SysReAllocString => AtlasSysReAllocString;
    guest SysAllocStringLen => AtlasSysAllocStringLen;
    guest SysReAllocStringLen => AtlasSysReAllocStringLen;
    guest SysFreeString => AtlasSysFreeString;
    guest SysStringLen => AtlasSysStringLen;
    guest VariantInit => AtlasVariantInit;
    guest VariantClear => AtlasVariantClear;
    guest VariantCopy => AtlasVariantCopy;
    missing VariantChangeType @ 12;
    missing SafeArrayCreate @ 15;
    missing SafeArrayDestroy;
    missing SafeArrayGetDim;
    missing SafeArrayGetUBound @ 19;
    missing SafeArrayGetLBound;
    missing SafeArrayAccessData @ 23;
    missing SafeArrayUnaccessData;
    missing SafeArrayPutElement @ 26;
    missing Ordinal77 @ 77;
    guest SysStringByteLen @ 149 => AtlasSysStringByteLen;
    guest SysAllocStringByteLen => AtlasSysAllocStringByteLen;
    // Not written yet: what larger programs import.

    missing LoadTypeLib @ 161;
    missing LoadRegTypeLib;
    missing LoadTypeLibEx @ 183;
    missing Ordinal185 @ 185;
    missing Ordinal186;
    missing VarUI4FromStr @ 277;
    missing VarBstrCat @ 313;
    missing VarBstrCmp;
    missing SafeArrayCreateVector @ 411;
}
