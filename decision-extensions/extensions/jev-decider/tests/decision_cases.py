"""Independent examples of the existing Jev request/answer contract.

Expected prompts are deliberately literal: changing a prompt changes what the
model is asked, even when the HTTP response shape stays the same. No expectation
imports a producer constant or calls the decision implementation.
"""

AUTO_INSTRUCTIONS = (
    "Choose the best branch for the current routing round's latest_user. It may continue the "
    "same user task, repeat prior text, or be a summary after a HiRoute rerouting boundary. "
    "Consider the task's complexity and the visible history of how prior branches performed. "
    "Preserve quality; use an economy branch only when it is appropriate for this work."
)
RULES_INSTRUCTIONS = (
    "Classify the current routing round's latest_user by task complexity. It may continue the "
    "same user task, repeat prior text, or be a summary. Use history only to resolve references "
    "and understand the actual work; do not call a simple task complex merely because a prior "
    "tool failed."
)
ASSESSMENT_QUESTION = {
    "type": "score",
    "instructions": (
        "Rate whether the branch used by the assessable tail of visible_conversation was competent "
        "for those routing rounds, including rounds with unknown terminal status. Consider progress, "
        "tool activity, failed and recovered attempts, round trips, and explicit feedback in latest_user. "
        "Repeated text, a summary, a continuation message, or silence is not by itself praise or a "
        "complaint. Do not rate the current not-yet-executed routing round."
    ),
    "criteria": [
        "The branch was not competent for the task: it failed to make useful progress, repeatedly made avoidable errors, or required substantial correction.",
        "The branch made useful but incomplete or uneven progress; the available evidence does not establish consistently competent execution.",
        "The branch was competent for the task: it advanced or completed the work reliably with an appropriate process and no material correction.",
    ],
}


def decision_cases() -> list[dict]:
    policy = {
        "smart_saving_simple": "Bounded changes following existing patterns.",
        "smart_saving_complex": "Uncertain root causes and architectural tradeoffs.",
    }
    generic = {"translate": "Translate prose.", "code": "Write code.", "explain": "Explain a concept."}
    previous = [{
        "branch_id": "smart_saving_simple",
        "executed_branch_id": "smart_saving_complex",
        "user": [{"kind": "text", "text": "Previous task"}],
        "status": "unknown",
        "steps": [[{"kind": "tool_activity", "tool": "apply_patch", "status": "completed"}]],
    }]
    examples = [
        ("auto_first_round", "auto", [], None,
         {"answers": {"branch": {"type": "choice", "choice": "smart_saving_simple"}}},
         {"branch_id": "smart_saving_simple"}),
        ("auto_assesses_previous_execution", "auto", previous, 0,
         {"answers": {"branch": {"type": "choice", "choice": "smart_saving_simple"},
                      "competence": {"type": "score", "score": 1.2}}},
         {"branch_id": "smart_saving_simple", "assessment": {"score": 0.6, "partial": False}}),
        ("rules_competence_guard", "rules", previous, 0,
         {"answers": {"complexity": {"type": "choice", "choice": "smart_saving_simple",
                                     "probabilities": {"smart_saving_simple": 0.95, "smart_saving_complex": 0.05}},
                      "competence": {"type": "score", "score": 0.2}}},
         {"branch_id": "smart_saving_complex", "assessment": {"score": 0.1, "partial": False}}),
        ("auto_selects_third_natural_language_branch", "auto", [], None,
         {"answers": {"branch": {"type": "choice", "choice": "explain"}}},
         {"branch_id": "explain"}),
        ("auto_keeps_extended_smart_saving_definitions", "auto", [], None,
         {"answers": {"branch": {"type": "choice", "choice": "explain"}}},
         {"branch_id": "explain"}),
    ]
    cases = []
    for name, mode, history, assessment_from, upstream, response in examples:
        custom = name in {
            "auto_selects_third_natural_language_branch",
            "auto_keeps_extended_smart_saving_definitions",
        }
        branches = generic if custom else policy
        if name == "auto_keeps_extended_smart_saving_definitions":
            # Adding a branch must disable the exact-binary policy override.
            branches = {
                "smart_saving_simple": "Keep this incoming simple description.",
                "smart_saving_complex": "Keep this incoming complex description.",
                "explain": "Explain a concept.",
            }
        request = {
            "branches": branches if custom else {
                "smart_saving_simple": "Incoming simple description.",
                "smart_saving_complex": "Incoming complex description.",
            },
            "latest_user": [{"kind": "text", "text": "Explain this change. 解释这个改动。"}],
            "visible_conversation": history,
            "history_partial": False,
            "assessment_from": assessment_from,
        }
        expected_questions = {
            "branch" if mode == "auto" else "complexity": {
                "type": "choice",
                "instructions": AUTO_INSTRUCTIONS if mode == "auto" else RULES_INSTRUCTIONS,
                "criteria": branches,
            },
        }
        if assessment_from is not None:
            expected_questions["competence"] = ASSESSMENT_QUESTION
        cases.append({
            "name": name,
            "mode": mode,
            "settings": {} if custom else {"branch_criteria": policy},
            "request": request,
            "upstream": upstream,
            "expected_upstream": {
                "model": "typesafe/jev-1.13",
                "state": {**request, "branches": branches},
                "questions": expected_questions,
            },
            "expected_response": response,
        })
    return cases
