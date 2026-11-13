//! iphlpapi.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "IPHLPAPI.dll";
    missing ConvertInterfaceGuidToLuid;
    missing GetAdaptersAddresses;
    missing GetAdaptersInfo;
    missing GetBestInterface;
    missing GetExtendedTcpTable;
    missing GetIfEntry2;
    missing NotifyAddrChange;
    // Not written yet: what larger programs import.
    missing CancelIPChangeNotify;
    missing FreeMibTable;
    missing GetIfTable2;
    missing GetInterfaceInfo;
    missing IpReleaseAddress;
    missing IpRenewAddress;
}
