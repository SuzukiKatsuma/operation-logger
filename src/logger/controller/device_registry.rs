use std::collections::HashMap;

#[derive(Debug, Default)]
pub(super) struct DeviceRegistry {
    ids: HashMap<isize, String>,
}

impl DeviceRegistry {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn device_id(&mut self, device_handle: isize) -> String {
        self.ids
            .entry(device_handle)
            .or_insert_with(|| format!("rawhid_{:016X}", device_handle as usize))
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_stable_session_device_ids_from_raw_input_handles() {
        let mut registry = DeviceRegistry::new();

        assert_eq!(
            registry.device_id(0x1234),
            "rawhid_0000000000001234".to_string()
        );
        assert_eq!(
            registry.device_id(0x1234),
            "rawhid_0000000000001234".to_string()
        );
    }

    #[test]
    fn assigns_different_ids_to_different_raw_input_handles() {
        let mut registry = DeviceRegistry::new();

        let first = registry.device_id(0x1234);
        let second = registry.device_id(0x5678);

        assert_ne!(first, second);
        assert_eq!(second, "rawhid_0000000000005678");
    }
}
