//! winusb.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "WINUSB.dll";
    missing WinUsb_ControlTransfer;
    missing WinUsb_Free;
    missing WinUsb_GetAssociatedInterface;
    missing WinUsb_GetOverlappedResult;
    missing WinUsb_Initialize;
    missing WinUsb_ReadPipe;
    missing WinUsb_ResetPipe;
    missing WinUsb_SetCurrentAlternateSetting;
    missing WinUsb_WritePipe;
}
