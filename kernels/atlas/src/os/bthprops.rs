//! bthprops.cpl: Bluetooth, declared so the programs that import it load; each export says it isn't written yet.

use super::dll;

dll! {
    "BTHPROPS.CPL";
    missing BluetoothEnableDiscovery;
    missing BluetoothEnableIncomingConnections;
    missing BluetoothFindDeviceClose;
    missing BluetoothFindFirstDevice;
    missing BluetoothFindFirstRadio;
    missing BluetoothFindNextDevice;
    missing BluetoothFindRadioClose;
    missing BluetoothGetRadioInfo;
    missing BluetoothIsConnectable;
    missing BluetoothSdpGetAttributeValue;
    missing BluetoothSdpGetContainerElementData;
}
