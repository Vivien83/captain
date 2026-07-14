#!/usr/bin/env python3
"""Example Captain agent: echoes back messages with a friendly greeting."""

import sys
import os

# Add parent directory to path for captain_sdk import
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from captain_sdk import Agent

agent = Agent()


@agent.on_message
def handle(message: str, context: dict) -> str:
    agent_id = context.get("agent_id", os.environ.get("CAPTAIN_AGENT_ID", "unknown"))
    return f"Hello from Python agent {agent_id}! You said: {message}"


agent.run()
