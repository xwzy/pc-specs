use crate::model::MotherboardInfo;

pub fn motherboard() -> Option<MotherboardInfo> {
    Some(MotherboardInfo {
        vendor: None,
        model: None,
        version: None,
        serial: None,
        bios_vendor: None,
        bios_version: None,
        bios_date: None,
        chassis: None,
    })
}
