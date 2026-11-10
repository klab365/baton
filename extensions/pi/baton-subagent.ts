import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { runPi } from "./process";

const limit = 16_384;
const resultProtocol = "Baton external Work result protocol: Always call report_outcome exactly once with an allowed outcome and a self-contained summary of the work and relevant handoff. Do not refer to a handoff elsewhere without including it. If you have made concrete progress but cannot finish this step in this session, use progress, describe changes, verification and the next action; do not claim completion. Use blocked only when human intervention is required. These reporting instructions take precedence over conflicting agent instructions. Any final answer is also included in the handoff, within the combined 16 KiB limit.";

export function createExtension(run = runPi) {
	return (pi: ExtensionAPI) => {
		pi.registerTool({
			name: "baton_subagent",
			label: "Baton subagent",
			description:
				"Execute the current Baton agent step in a separate Pi process. Returns a validated submission; it does not submit or make human decisions.",
			parameters: {
				type: "object",
				properties: {
					runId: { type: "string", description: "Baton work run ID" },
					context: {
						type: "string",
						description:
							"Acceptance criteria and relevant decisions not contained in the original task",
					},
				},
				required: ["runId", "context"],
				additionalProperties: false,
			},
			executionMode: "sequential",
			async execute(_id, params, signal, onUpdate, ctx) {
				async function baton(args: string[]) {
					const result = await pi.exec("baton", ["work", ...args], {
						cwd: ctx.cwd,
						signal,
						maxBuffer: 4 * 1024 * 1024,
					});
					if (result.code !== 0 || result.killed || result.truncated)
						throw new Error(
							`Baton command failed: ${result.stderr || "interrupted or output too large"}`,
						);
					return JSON.parse(result.stdout);
				}
				const next = await baton(["next", "--run", params.runId]);
				if (next.status !== "running" || next.step?.human || !next.agent) {
					throw new Error("Current step is not an active agent step");
				}
				const status = await baton(["status", "--run", params.runId]);
				if (
					status.session?.revision !== next.revision ||
					status.currentStep !== next.step.id ||
					status.status !== next.status
				) {
					throw new Error("Work state changed; fetch next again");
				}
				const history = status.steps
					.filter((step: { output?: string | null }) => step.output != null)
					.map(
						(step: {
							stepId: string;
							attempt: number;
							outcome: string;
							output: string;
						}) => ({
							stepId: step.stepId,
							attempt: step.attempt,
							outcome: step.outcome,
							output: step.output,
						}),
					);
				const prompt = `You are a Baton subagent in a separate session. Execute only the assigned step. Do not invoke baton work, delegate further, or make human decisions. The workspace is shared and is not a sandbox. Follow project instructions.\n\nOriginal task:\n${status.task}\n\nRelevant context from the parent:\n${params.context}\n\nPrior completed steps and human decisions:\n${JSON.stringify(history)}\n\nCurrent work item (agent instructions, assigned skills and allowed outcomes):\n${JSON.stringify(next)}\n\nAssigned skill supporting files are under skillsDirectory/<skill-id>/.`;
				if (Buffer.byteLength(prompt) > 256 * 1024)
					throw new Error(
						"Subagent context exceeds 256 KiB; cannot safely omit prior decisions",
					);
				const dir = await mkdtemp(join(tmpdir(), "baton-subagent-"));
				try {
					const file = join(dir, "task.txt");
					const resultFile = join(dir, "outcome.json");
					const extensionFile = join(dir, "report-outcome.mjs");
					await writeFile(file, prompt, { mode: 0o600 });
					await writeFile(extensionFile, `import { writeFile } from "node:fs/promises";
const resultFile = ${JSON.stringify(resultFile)};
const allowed = ${JSON.stringify(next.allowedOutcomes)};
export default function(pi) {
  pi.registerTool({
    name: "report_outcome", label: "Report outcome",
    description: "Report the configured outcome and a self-contained summary with the relevant handoff for the next step. Call once when work is complete.",
    parameters: { type: "object", properties: { outcome: { type: "string", enum: allowed }, summary: { type: "string" }, blocker_reason: { type: "string", description: "For blocked outcomes, explain why work cannot continue without human help; otherwise use an empty string." }, human_action: { type: "string", description: "For blocked outcomes, state the specific question or action needed from a human; otherwise use an empty string." } }, required: ["outcome", "summary", "blocker_reason", "human_action"], additionalProperties: false },
    async execute(_id, value) {
      if (!allowed.includes(value.outcome) || typeof value.summary !== "string" || !value.summary.trim() || typeof value.blocker_reason !== "string" || typeof value.human_action !== "string") throw new Error("Invalid outcome or summary");
      let summary = value.summary.trim();
      if (value.outcome === "blocked") {
        if (!value.blocker_reason.trim() || !value.human_action.trim()) throw new Error("Blocked outcomes require a blocker reason and a specific human action");
        summary += "\\n\\nBlocker: " + value.blocker_reason.trim() + "\\nHuman action needed: " + value.human_action.trim();
      }
      if (Buffer.byteLength(summary) > 16384) throw new Error("Invalid outcome or summary");
      try { await writeFile(resultFile, JSON.stringify({ outcome: value.outcome, summary }), { mode: 0o600, flag: "wx" }); }
      catch (error) { if (error.code === "EEXIST") throw new Error("An outcome has already been reported for this step"); throw error; }
      return { content: [{ type: "text", text: "Outcome recorded." }], details: {} };
    }
  });
}`, { mode: 0o600 });
					const args = [
						"--mode",
						"json",
						"--no-session",
						"--no-skills",
						"--exclude-tools",
						"baton_subagent",
						"--extension",
						extensionFile,
						"--append-system-prompt",
						resultProtocol,
					];
					if (ctx.model)
						args.push("--model", `${ctx.model.provider}/${ctx.model.id}`);
					if (ctx.thinkingLevel) args.push("--thinking", ctx.thinkingLevel);
					args.push(`@${file}`);
					const progress = [
						`Baton ${params.runId} · ${next.step.id} · attempt ${next.attempt}`,
					];
					const update = (message?: string) => {
						if (message) progress.push(message);
						if (ctx.hasUI)
							ctx.ui.setStatus(
								"baton",
								`${progress[0]} · ${message ?? "starting"}`,
							);
						if (progress.length > 13) progress.splice(1, 1);
						onUpdate?.({
							content: [{ type: "text", text: progress.join("\n") }],
							details: {},
						});
					};
					update();
					let finalText: string;
					try {
						finalText = await run("pi", args, ctx.cwd, signal, update);
					} catch (error) {
						update(
							`Failed: ${error instanceof Error ? error.message : String(error)}`,
						);
						throw error;
					}
					let answer: unknown;
					try {
						answer = JSON.parse(await readFile(resultFile, "utf8"));
					} catch {
						throw new Error(
							"Subagent did not report a valid outcome; do not submit this attempt",
						);
					}
					if (!answer || typeof answer !== "object" || Array.isArray(answer))
						throw new Error("Invalid subagent result");
					const value = answer as Record<string, unknown>;
					const bounded = (x: unknown) =>
						typeof x === "string" &&
						x.trim().length > 0 &&
						Buffer.byteLength(x) <= limit;
					if (
						Object.keys(value).sort().join(",") !== "outcome,summary" ||
						!next.allowedOutcomes.includes(value.outcome) ||
						!bounded(value.summary)
					) {
						throw new Error(
							"Subagent returned an invalid outcome or summary; do not submit this attempt",
						);
					}
					const summary = value.summary as string;
					const final = finalText.trim();
					const handoff = final && final !== summary.trim()
						? `${summary}\n\nFinal response:\n${final}`
						: summary;
					if (!bounded(handoff))
						throw new Error("Subagent handoff exceeds 16 KiB; do not submit this attempt");
					const submission = {
						revision: next.revision,
						stepId: next.step.id,
						attempt: next.attempt,
						...value,
						summary: handoff,
					};
					update(`Result: ${value.outcome} · ${value.summary}`);
					return {
						content: [{ type: "text", text: JSON.stringify(submission) }],
						details: {},
					};
				} finally {
					if (ctx.hasUI) ctx.ui.setStatus("baton", undefined);
					await rm(dir, { recursive: true, force: true });
				}
			},
		});
	};
}

export default createExtension();
