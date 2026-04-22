/// 任务状态机与并发调度（架构 §8）
use crate::infra::AppError;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Semaphore;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Processing,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInvocation {
    pub plugin_id: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertTask {
    pub task_id: String,
    pub status: TaskStatus,
    pub progress: u8,
    pub input_format: Option<String>,
    pub output_format: String,
    pub plugin_chain: Vec<PluginInvocation>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<TaskError>,
    /// 原始上传文件展示名（仅 basename，无路径），供任务列表区分多任务
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_filename_hint: Option<String>,
    /// 下载结果时使用的文件名（由上传 basename + 输出扩展名派生，如 `a.docx` → `a.md`）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_download_filename: Option<String>,
}

impl ConvertTask {
    pub fn new(output_format: String, input_format: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            task_id: Uuid::new_v4().to_string(),
            status: TaskStatus::Pending,
            progress: 0,
            input_format,
            output_format,
            plugin_chain: vec![],
            created_at: now,
            updated_at: now,
            result_url: None,
            error: None,
            input_filename_hint: None,
            result_download_filename: None,
        }
    }

    pub fn set_processing(&mut self) {
        self.status = TaskStatus::Processing;
        self.updated_at = Utc::now();
    }

    pub fn set_completed(&mut self, result_url: String) {
        self.status = TaskStatus::Completed;
        self.progress = 100;
        self.result_url = Some(result_url);
        self.updated_at = Utc::now();
    }

    pub fn set_failed(&mut self, err: AppError) {
        self.status = TaskStatus::Failed;
        self.error = Some(TaskError {
            code: err.error_code().to_string(),
            message: err.to_string(),
            details: err.to_response().details,
        });
        self.updated_at = Utc::now();
    }

    pub fn set_cancelled(&mut self) {
        self.status = TaskStatus::Cancelled;
        self.updated_at = Utc::now();
    }

    pub fn set_progress(&mut self, progress: u8) {
        self.progress = progress;
        self.updated_at = Utc::now();
    }
}

/// 任务仓储 + 并发控制
#[derive(Clone)]
pub struct TaskManager {
    tasks: Arc<DashMap<String, ConvertTask>>,
    semaphore: Arc<Semaphore>,
    max_concurrent: usize,
}

impl TaskManager {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            tasks: Arc::new(DashMap::new()),
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            max_concurrent,
        }
    }

    pub fn create_task(&self, output_format: String, input_format: Option<String>) -> ConvertTask {
        let task = ConvertTask::new(output_format, input_format);
        self.tasks.insert(task.task_id.clone(), task.clone());
        task
    }

    pub fn get_task(&self, task_id: &str) -> Option<ConvertTask> {
        self.tasks.get(task_id).map(|t| t.clone())
    }

    pub fn update_task<F>(&self, task_id: &str, f: F) -> Result<(), AppError>
    where
        F: FnOnce(&mut ConvertTask),
    {
        match self.tasks.get_mut(task_id) {
            Some(mut t) => {
                f(&mut t);
                Ok(())
            }
            None => Err(AppError::TaskNotFound {
                task_id: task_id.to_string(),
            }),
        }
    }

    pub fn cancel_task(&self, task_id: &str) -> Result<(), AppError> {
        self.update_task(task_id, |t| {
            if matches!(t.status, TaskStatus::Pending | TaskStatus::Processing) {
                t.set_cancelled();
            }
        })
    }

    /// 从内存中移除任务记录（用于用户删除已完成/失败/已取消的任务）。
    pub fn remove_task_record(&self, task_id: &str) -> Option<ConvertTask> {
        self.tasks.remove(task_id).map(|(_k, v)| v)
    }

    pub fn list_tasks(&self) -> Vec<ConvertTask> {
        let mut tasks: Vec<ConvertTask> = self.tasks.iter().map(|e| e.clone()).collect();
        tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        tasks
    }

    pub fn processing_count(&self) -> usize {
        self.max_concurrent - self.semaphore.available_permits()
    }

    /// 申请执行 slot（并发上限 10）
    pub async fn acquire_slot(&self) -> Result<tokio::sync::SemaphorePermit<'_>, AppError> {
        self.semaphore
            .acquire()
            .await
            .map_err(|_| AppError::Internal("Semaphore closed".into()))
    }

    /// 垃圾回收过期任务（简单策略：Completed/Failed 超 TTL 则清理元数据引用）
    pub fn gc_expired(&self, ttl_secs: u64) {
        let now = Utc::now();
        let expired: Vec<String> = self
            .tasks
            .iter()
            .filter(|e| {
                matches!(
                    e.status,
                    TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
                ) && (now - e.updated_at).num_seconds() as u64 > ttl_secs
            })
            .map(|e| e.task_id.clone())
            .collect();
        for id in expired {
            self.tasks.remove(&id);
            tracing::debug!(task_id = %id, "GC: removed expired task");
        }
    }
}
