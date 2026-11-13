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
    guest SysStringByteLen @ 149 => AtlasSysStringByteLen;
    guest SysAllocStringByteLen => AtlasSysAllocStringByteLen;
}
