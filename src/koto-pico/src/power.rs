//! PicoCalc STM32 battery-register decoding.

use koto_core::hal::PowerState;

pub const VERSION_REGISTER: u8 = 0x01;
pub const BATTERY_REGISTER: u8 = 0x0b;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FirmwareVersion {
    pub raw: u8,
    pub major: u8,
    pub minor: u8,
}

pub const fn decode_version(response: [u8; 2]) -> Option<FirmwareVersion> {
    if response[0] != 0 || response[1] == 0 {
        return None;
    }
    Some(FirmwareVersion {
        raw: response[1],
        major: response[1] >> 4,
        minor: response[1] & 0x0f,
    })
}

pub const fn decode_battery(response: [u8; 2]) -> Option<PowerState> {
    if response[0] != BATTERY_REGISTER || response[1] == 0 {
        return None;
    }
    let charging = response[1] & 0x80 != 0;
    let percent = response[1] & 0x7f;
    if percent > 100 {
        return None;
    }
    Some(if charging {
        PowerState::charging(Some(percent), None)
    } else {
        PowerState::percent(percent, None)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_bios_version_nibbles() {
        assert_eq!(
            decode_version([0, 0x16]),
            Some(FirmwareVersion {
                raw: 0x16,
                major: 1,
                minor: 6,
            })
        );
    }

    #[test]
    fn maps_valid_percentage_without_inventing_voltage() {
        assert_eq!(
            decode_battery([BATTERY_REGISTER, 73]),
            Some(PowerState::percent(73, None))
        );
    }

    #[test]
    fn maps_charging_flag_from_bit_seven() {
        assert_eq!(
            decode_battery([BATTERY_REGISTER, 0x80 | 42]),
            Some(PowerState::charging(Some(42), None))
        );
    }

    #[test]
    fn rejects_wrong_marker_zero_and_invalid_percentage() {
        assert_eq!(decode_battery([0, 50]), None);
        assert_eq!(decode_battery([BATTERY_REGISTER, 0]), None);
        assert_eq!(decode_battery([BATTERY_REGISTER, 127]), None);
    }

    #[test]
    fn rejects_zero_firmware_version_as_unprepared_response() {
        assert_eq!(decode_version([0, 0]), None);
    }
}
