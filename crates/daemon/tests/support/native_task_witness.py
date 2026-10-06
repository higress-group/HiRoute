"""Exact fixture-owned cancelled task identity; never a production recovery reader."""
import json
from pathlib import Path
import sqlite3


def cancelled_session(fixture, run_id, harness):
    """Cancellation has no successful-Continue binding. Observe its exact stored identity."""
    database = Path(fixture['product_storage']) / 'live/runtime.db'
    assert not database.is_symlink() and database.is_file()
    with sqlite3.connect(database.as_uri() + '?mode=ro', uri=True) as connection:
        task_row = connection.execute(
            'SELECT record_json FROM delegation_tasks WHERE workspace_id=? AND task_id=?',
            ('personal/default', fixture['task_id'])).fetchone()
        run_row = connection.execute(
            'SELECT record_json FROM delegation_runs WHERE workspace_id=? AND run_id=?',
            ('personal/default', run_id)).fetchone()
    assert task_row is not None and run_row is not None, 'exact cancelled task/run record is missing'
    task, run = json.loads(task_row[0]), json.loads(run_row[0])
    assert (task['task_id'] == run['task_id'] == fixture['task_id']
            and task['latest_run_id'] == run['run_id'] == run_id
            and task['plan']['harness'] == harness
            and run['progress']['state'] == 'cancelled'
            and run['progress']['cleanup'] == 'complete'), 'exact cancelled task identity or cleanup mismatch'
    session = task['session']
    assert (session and isinstance(session.get('native_session_id'), str)
            and session['native_session_id'] == session['acp_session_id']), \
        'cancelled native task lacks one exact native session identity'
    return session['native_session_id']
