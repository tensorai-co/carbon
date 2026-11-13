//! winspool.drv: printing, declared so the programs that import it load; each export says it isn't written yet.

use super::dll;

dll! {
    "WINSPOOL.DRV";
    missing ClosePrinter;
    missing DeviceCapabilitiesW;
    missing DocumentPropertiesW;
    missing EnumPrintersW;
    missing GetPrinterDriverW;
    missing GetPrinterW;
    missing OpenPrinterW;
    missing Ordinal203 @ 203;
}
