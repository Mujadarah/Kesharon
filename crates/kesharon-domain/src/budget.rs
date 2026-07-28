use crate::TaskError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceBudget {
    max_memory_bytes: u64,
    max_disk_write_bytes: u64,
    max_concurrent_tools: u16,
}

impl ResourceBudget {
    pub fn new(
        max_memory_bytes: u64,
        max_disk_write_bytes: u64,
        max_concurrent_tools: u16,
    ) -> Result<Self, TaskError> {
        if max_memory_bytes == 0 || max_disk_write_bytes == 0 || max_concurrent_tools == 0 {
            return Err(TaskError::InvalidResourceBudget);
        }

        Ok(Self {
            max_memory_bytes,
            max_disk_write_bytes,
            max_concurrent_tools,
        })
    }

    pub const fn max_memory_bytes(self) -> u64 {
        self.max_memory_bytes
    }

    pub const fn max_disk_write_bytes(self) -> u64 {
        self.max_disk_write_bytes
    }

    pub const fn max_concurrent_tools(self) -> u16 {
        self.max_concurrent_tools
    }
}
