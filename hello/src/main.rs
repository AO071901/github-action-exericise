use axum::{
    routing::{get, post},
    http::StatusCode,
    Json, Router,
    extract::{State, Path},
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use dotenv::dotenv;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Task {
    id: Uuid,
    title: String,
    completed: bool,
    priority: Option<String>,
    estimated_time: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateTask {
    title: String,
}

type Db = Arc<Mutex<Vec<Task>>>;

#[tokio::main]
async fn main() {
    dotenv().ok();
    let db = Arc::new(Mutex::new(Vec::new()));

    let app = Router::new()
        .route("/tasks", 
            get(|state: State<Db>| async move { list_tasks(state).await })
            .post(|state: State<Db>, payload: Json<CreateTask>| async move {
                create_task(state, payload).await
            })
        )
        .route("/tasks/:id", 
            get(|state: State<Db>, path: Path<Uuid>| async move { get_task(state, path).await })
            .patch(|state: State<Db>, path: Path<Uuid>, payload: Json<Task>| async move { update_task(state, path, payload).await })
            .delete(|state: State<Db>, path: Path<Uuid>| async move { delete_task(state, path).await })
        )
        .with_state(db);

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn list_tasks(State(db): State<Db>) -> Json<Vec<Task>> {
    let tasks = db.lock().unwrap().clone();
    Json(tasks)
}

async fn create_task(
    State(db): State<Db>,
    Json(payload): Json<CreateTask>,
) -> (StatusCode, Json<Task>) {
    let task = Task {
        id: Uuid::new_v4(),
        title: payload.title,
        completed: false,
        priority: None,
        estimated_time: None,
    };
    
    let task_with_ai = match analyze_task_with_claude(task.clone()).await {
        Ok(analyzed_task) => analyzed_task,
        Err(_) => task.clone(),
    };

    // MutexGuardの生存期間を短くするため、スコープを制限します
    {
        let mut db_guard = db.lock().unwrap();
        db_guard.push(task_with_ai.clone());
    }

    (StatusCode::CREATED, Json(task_with_ai))
}

async fn get_task(
    State(db): State<Db>,
    Path(id): Path<Uuid>,
) -> Result<Json<Task>, StatusCode> {
    let db = db.lock().unwrap();
    db.iter()
        .find(|task| task.id == id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_task(
    State(db): State<Db>,
    Path(id): Path<Uuid>,
    Json(payload): Json<Task>,
) -> Result<Json<Task>, StatusCode> {
    let mut db = db.lock().unwrap();
    if let Some(task) = db.iter_mut().find(|t| t.id == id) {
        *task = payload;
        Ok(Json(task.clone()))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn delete_task(
    State(db): State<Db>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    let mut db = db.lock().unwrap();
    let len = db.len();
    db.retain(|t| t.id != id);
    if db.len() != len {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn analyze_task_with_claude(task: Task) -> Result<Task, StatusCode> {
    let claude_api_key = std::env::var("CLAUDE_API_KEY").expect("CLAUDE_API_KEY must be set");
    let client = reqwest::Client::new();
    let prompt = format!(
        "Analyze the following task and suggest a priority level (High, Medium, Low) and estimated time to complete (in hours): {}",
        task.title
    );

    let response = client
        .post("https://api.anthropic.com/v1/completions")
        .header("Content-Type", "application/json")
        .header("X-API-Key", claude_api_key)
        .json(&serde_json::json!({
            "model": "claude-2",
            "prompt": prompt,
            "max_tokens_to_sample": 150,
        }))
        .send()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .json::<serde_json::Value>()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let ai_response = response["completion"].as_str().unwrap_or("");
    let mut task = task;
    if ai_response.contains("High") {
        task.priority = Some("High".to_string());
    } else if ai_response.contains("Medium") {
        task.priority = Some("Medium".to_string());
    } else if ai_response.contains("Low") {
        task.priority = Some("Low".to_string());
    }

    if let Some(time) = ai_response.split("hours").next().and_then(|s| s.split_whitespace().last()) {
        task.estimated_time = Some(format!("{} hours", time));
    }

    Ok(task)
}