use crate::domain::tasks::Task;

pub trait TaskOperationLease: Send {}

pub trait TaskRepository: Send + Sync {
    /// Reserve a task while a run or task mutation is active. The lease releases on drop.
    fn acquire_operation_lease(&self, id: &str) -> Result<Box<dyn TaskOperationLease>, String>;
    fn load(&self, id: &str) -> Result<Task, String>;
    fn list(&self) -> Result<Vec<Task>, String>;
    fn save(&self, task: &Task) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
}
