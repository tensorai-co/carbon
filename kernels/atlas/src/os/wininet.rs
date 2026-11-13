//! wininet.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "WININET.dll";
    missing HttpAddRequestHeadersW;
    missing HttpOpenRequestW;
    missing HttpQueryInfoW;
    missing HttpSendRequestW;
    missing InternetCloseHandle;
    missing InternetConnectW;
    missing InternetCrackUrlW;
    missing InternetOpenW;
    missing InternetQueryDataAvailable;
    missing InternetReadFile;
    missing InternetSetOptionW;
    // Not written yet: what larger programs import.
    missing GetUrlCacheEntryInfoExW;
}
