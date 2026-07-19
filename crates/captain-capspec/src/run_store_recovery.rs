use super::*;

impl RunStore {
    pub(crate) fn interrupt_run(
        &mut self,
        run_id: &str,
        reason: &str,
    ) -> Result<CapabilityRunStatus, ExecutorError> {
        let running = {
            let mut statement = self.connection.prepare(
                "SELECT step_id, replay_safe FROM capspec_run_nodes
                 WHERE run_id = ?1 AND status = 'running' ORDER BY ordinal",
            )?;
            let rows = statement
                .query_map([run_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        for (step_id, replay_safe) in running {
            if replay_safe {
                self.reset_node_pending(run_id, &step_id)?;
            } else {
                self.mark_node_uncertain(run_id, &step_id, reason)?;
            }
        }
        self.refresh_run_status(run_id, Some(reason))
    }

    pub(crate) fn refresh_run_status(
        &mut self,
        run_id: &str,
        error: Option<&str>,
    ) -> Result<CapabilityRunStatus, ExecutorError> {
        let (uncertain, failed, pending, running): (usize, usize, usize, usize) =
            self.connection.query_row(
                "SELECT
                    SUM(CASE WHEN status = 'uncertain' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END)
                 FROM capspec_run_nodes WHERE run_id = ?1",
                [run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
        let status = if uncertain > 0 {
            CapabilityRunStatus::WaitingDecision
        } else if failed > 0 {
            CapabilityRunStatus::Failed
        } else if running > 0 {
            CapabilityRunStatus::Running
        } else if pending > 0 {
            CapabilityRunStatus::Interrupted
        } else {
            CapabilityRunStatus::Interrupted
        };
        self.set_run_status(run_id, status, None, error)?;
        Ok(status)
    }

    pub(super) fn recover_interrupted_runs(&mut self) -> Result<(), ExecutorError> {
        let transaction = self.connection.transaction()?;
        let run_ids = {
            let mut statement = transaction.prepare(
                "SELECT run_id FROM capspec_runs WHERE status = 'running' ORDER BY run_id",
            )?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        for run_id in run_ids {
            transaction.execute(
                "UPDATE capspec_run_nodes
                 SET status = CASE WHEN replay_safe = 1 THEN 'pending' ELSE 'uncertain' END,
                     attempts = CASE
                         WHEN replay_safe = 1 THEN MAX(attempts - 1, 0)
                         ELSE attempts
                     END,
                     error_blob = NULL, finished_at = NULL
                 WHERE run_id = ?1 AND status = 'running'",
                [&run_id],
            )?;
            let uncertain: usize = transaction.query_row(
                "SELECT COUNT(*) FROM capspec_run_nodes
                 WHERE run_id = ?1 AND status = 'uncertain'",
                [&run_id],
                |row| row.get(0),
            )?;
            let status = if uncertain == 0 {
                CapabilityRunStatus::Interrupted
            } else {
                CapabilityRunStatus::WaitingDecision
            };
            transaction.execute(
                "UPDATE capspec_runs
                 SET status = ?2,
                     operator_resume_state = CASE
                         WHEN ?2 = 'waiting_decision' THEN 'none'
                         WHEN operator_resume_state = 'in_progress' THEN 'requested'
                         ELSE operator_resume_state
                     END,
                     updated_at = ?3
                 WHERE run_id = ?1",
                params![run_id, status.as_storage(), crate::store::now()],
            )?;
        }
        transaction.execute(
            "UPDATE capspec_runs
             SET operator_resume_state = CASE
                 WHEN status IN ('succeeded', 'failed', 'waiting_decision') THEN 'none'
                 WHEN operator_resume_state = 'in_progress' THEN 'requested'
                 ELSE operator_resume_state
             END
             WHERE operator_resume_state != 'none'",
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }
}
