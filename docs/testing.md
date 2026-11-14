# Manual testing

## Pi agent session resumption

This test checks that the same Pi agent remembers a previous conversation after a human pause and a Pi restart. Create `.baton/pipelines/pi-session-test.yml` with the following content, replacing `factory-builder` with the ID of a configured Pi agent if necessary:

```yaml
id: pi-session-test
steps:
  - id: remember
    name: Remember code
    type: agent
    agentId: factory-builder
    includeTask: true
    prompt: "Remember the code from the task and confirm it in your answer."
  - id: pause
    name: Human pause
    type: human
  - id: recall
    name: Recall code
    type: agent
    agentId: factory-builder
    includeTask: false
    prompt: "What code did you remember in the first step? Answer only with the code."
```

1. From the repository root, run `mise run dev-pi`. This builds the CLI and starts Pi with the local Baton extension. The configured Pi model must have working authentication; model calls may incur costs.
2. Enter `/baton`, choose `pi-session-test`, and use this task:

   ```text
   Merke dir den Code BLAUER-FALKE-731.
   ```

3. Wait until the pipeline reaches its human decision. Note the run ID and leave the decision pending. Exit Pi.
4. Start Pi again with `mise run dev-pi` and enter `/baton resume RUN_ID`, replacing `RUN_ID` with the recorded ID.
5. Choose the `responded` decision and enter only `WEITER` as the response. Do not repeat the code.
6. Check the final agent step's answer: it should be `BLAUER-FALKE-731`.

The human response is the new handoff, and the final step has `includeTask: false`. The code is therefore not supplied again in the new step input; it must come from the agent's resumed conversation. If the run fails or returns a different answer, record the run ID and error or final answer for investigation. Do not include credentials or other secrets in task text or reports.

This test does not verify the follow-up behavior for a missing `report_outcome`. The test pipeline can be removed with `rm .baton/pipelines/pi-session-test.yml` when no longer needed.
