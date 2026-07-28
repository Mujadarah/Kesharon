use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::{ResourceBudget, TaskId, TaskStepId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskState {
    Draft,
    Planning,
    AwaitingApproval,
    Executing,
    Paused,
    AwaitingReview,
    Completed,
    Failed,
    Cancelled,
}

impl TaskState {
    const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskStep {
    id: TaskStepId,
    description: String,
}

impl TaskStep {
    pub fn new(id: TaskStepId, description: impl Into<String>) -> Result<Self, TaskError> {
        let description = description.into();
        if description.trim().is_empty() {
            return Err(TaskError::EmptyStepDescription);
        }
        Ok(Self { id, description })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskPlan {
    steps: Vec<TaskStep>,
}

impl TaskPlan {
    pub fn new(steps: Vec<TaskStep>) -> Result<Self, TaskError> {
        if steps.is_empty() {
            return Err(TaskError::EmptyPlan);
        }
        Ok(Self { steps })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Task {
    id: TaskId,
    goal: String,
    budget: ResourceBudget,
    state: TaskState,
    plan: Option<TaskPlan>,
}

impl Task {
    pub fn new(
        id: TaskId,
        goal: impl Into<String>,
        budget: ResourceBudget,
    ) -> Result<Self, TaskError> {
        let goal = goal.into();
        if goal.trim().is_empty() {
            return Err(TaskError::EmptyGoal);
        }

        Ok(Self {
            id,
            goal,
            budget,
            state: TaskState::Draft,
            plan: None,
        })
    }

    pub const fn state(&self) -> TaskState {
        self.state
    }

    pub fn start_planning(&mut self) -> Result<(), TaskError> {
        self.transition(TaskState::Draft, TaskState::Planning, "start_planning")
    }

    pub fn request_approval(&mut self, plan: TaskPlan) -> Result<(), TaskError> {
        self.require_state(TaskState::Planning, "request_approval")?;
        self.plan = Some(plan);
        self.state = TaskState::AwaitingApproval;
        Ok(())
    }

    pub fn approve(&mut self) -> Result<(), TaskError> {
        self.transition(TaskState::AwaitingApproval, TaskState::Executing, "approve")
    }

    pub fn reject(&mut self) -> Result<(), TaskError> {
        self.transition(TaskState::AwaitingApproval, TaskState::Cancelled, "reject")
    }

    pub fn pause(&mut self) -> Result<(), TaskError> {
        self.transition(TaskState::Executing, TaskState::Paused, "pause")
    }

    pub fn resume(&mut self) -> Result<(), TaskError> {
        self.transition(TaskState::Paused, TaskState::Executing, "resume")
    }

    pub fn submit_for_review(&mut self) -> Result<(), TaskError> {
        self.transition(
            TaskState::Executing,
            TaskState::AwaitingReview,
            "submit_for_review",
        )
    }

    pub fn request_revision(&mut self) -> Result<(), TaskError> {
        self.transition(
            TaskState::AwaitingReview,
            TaskState::Executing,
            "request_revision",
        )
    }

    pub fn accept(&mut self) -> Result<(), TaskError> {
        self.transition(TaskState::AwaitingReview, TaskState::Completed, "accept")
    }

    fn transition(
        &mut self,
        from: TaskState,
        to: TaskState,
        action: &'static str,
    ) -> Result<(), TaskError> {
        self.require_state(from, action)?;
        self.state = to;
        Ok(())
    }

    fn require_state(&self, expected: TaskState, action: &'static str) -> Result<(), TaskError> {
        if self.state.is_terminal() {
            return Err(TaskError::TerminalState);
        }
        if self.state != expected {
            return Err(TaskError::InvalidTransition {
                from: self.state,
                action,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskError {
    EmptyIdentifier,
    EmptyGoal,
    EmptyPlan,
    EmptyStepDescription,
    InvalidResourceBudget,
    InvalidTransition {
        from: TaskState,
        action: &'static str,
    },
    TerminalState,
}

impl Display for TaskError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyIdentifier => formatter.write_str("identifier must not be blank"),
            Self::EmptyGoal => formatter.write_str("task goal must not be blank"),
            Self::EmptyPlan => formatter.write_str("task plan must contain at least one step"),
            Self::EmptyStepDescription => {
                formatter.write_str("task step description must not be blank")
            }
            Self::InvalidResourceBudget => {
                formatter.write_str("resource budget ceilings must be greater than zero")
            }
            Self::InvalidTransition { from, action } => {
                write!(formatter, "cannot {action} while task is {from:?}")
            }
            Self::TerminalState => formatter.write_str("terminal task state cannot be changed"),
        }
    }
}

impl Error for TaskError {}
