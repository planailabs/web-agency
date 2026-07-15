-- action_template_runs moved into the plan-ai-actions crate's own migration
-- chain (_plan_ai_actions_migrations), which recreates it right after this
-- chain runs. The table held no data worth keeping at the time of the move.
DROP TABLE IF EXISTS action_template_runs;
