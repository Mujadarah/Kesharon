use crate::TaskError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceBudget {
    max_memory_bytes: u64,
    max_disk_write_bytes: u64,
    max_concurrent_tools: u16,
    max_prompt_tokens: Option<u64>,
    max_completion_tokens: Option<u64>,
    max_cost_micros: Option<u64>,
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
            max_prompt_tokens: None,
            max_completion_tokens: None,
            max_cost_micros: None,
        })
    }

    pub fn with_token_limits(
        mut self,
        max_prompt_tokens: u64,
        max_completion_tokens: u64,
        max_cost_micros: u64,
    ) -> Result<Self, TaskError> {
        if max_prompt_tokens == 0 || max_completion_tokens == 0 || max_cost_micros == 0 {
            return Err(TaskError::InvalidResourceBudget);
        }
        self.max_prompt_tokens = Some(max_prompt_tokens);
        self.max_completion_tokens = Some(max_completion_tokens);
        self.max_cost_micros = Some(max_cost_micros);
        Ok(self)
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

    pub const fn max_prompt_tokens(self) -> Option<u64> {
        self.max_prompt_tokens
    }

    pub const fn max_completion_tokens(self) -> Option<u64> {
        self.max_completion_tokens
    }

    pub const fn max_cost_micros(self) -> Option<u64> {
        self.max_cost_micros
    }
}
