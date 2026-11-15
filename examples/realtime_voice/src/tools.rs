//! Tool definitions for the realtime voice example.
//!
//! These are the function schemas registered with the realtime session,
//! allowing the voice model to invoke tools during a live conversation.

use adk_realtime::config::ToolDefinition;
use serde_json::json;

/// Weather lookup tool definition.
///
/// The model calls this when the user asks about weather conditions.
/// In a real application, this would query a weather API.
pub fn get_weather_tool_def() -> ToolDefinition {
    ToolDefinition {
        name: "get_weather".to_string(),
        description: Some("Get current weather conditions for a city".to_string()),
        parameters: Some(json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "string",
                    "description": "City name (e.g., 'San Francisco', 'Tokyo')"
                }
            },
            "required": ["city"]
        })),
    }
}

/// Flight search tool definition.
///
/// The model calls this when the user asks about flights between cities.
/// In a real application, this would query a flights API.
#[allow(dead_code)]
pub fn search_flights_tool_def() -> ToolDefinition {
    ToolDefinition {
        name: "search_flights".to_string(),
        description: Some("Search for available flights between two cities".to_string()),
        parameters: Some(json!({
            "type": "object",
            "properties": {
                "from": {
                    "type": "string",
                    "description": "Departure city"
                },
                "to": {
                    "type": "string",
                    "description": "Arrival city"
                },
                "date": {
                    "type": "string",
                    "description": "Travel date (YYYY-MM-DD format, optional)"
                }
            },
            "required": ["from", "to"]
        })),
    }
}

/// Timer/reminder tool definition.
///
/// Useful for meditation and mindfulness sessions — the model can set
/// timers for breathing exercises.
#[allow(dead_code)]
pub fn set_timer_tool_def() -> ToolDefinition {
    ToolDefinition {
        name: "set_timer".to_string(),
        description: Some("Set a timer for a specified number of seconds".to_string()),
        parameters: Some(json!({
            "type": "object",
            "properties": {
                "seconds": {
                    "type": "integer",
                    "description": "Duration in seconds"
                },
                "label": {
                    "type": "string",
                    "description": "Label for the timer (e.g., 'breathing exercise')"
                }
            },
            "required": ["seconds"]
        })),
    }
}
