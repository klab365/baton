import { expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { createExtension } from "./baton-subagent";
import { runPi } from "./process";

type TestTool = {
	execute: (...args: unknown[]) => Promise<{ content: { text: string }[] }>;
};

function fixture(
	answer: string,
	options: { human?: boolean; code?: number; stale?: boolean; exerciseTool?: boolean; realPi?: boolean } = {},
) {
	let tool: TestTool | undefined;
	const calls: string[][] = [];
	const statuses: (string | undefined)[] = [];
	const pi = {
		registerTool(value: TestTool) {
			tool = value;
		},
		async exec(command: string, args: string[], opts: { cwd: string }) {
			calls.push([command, ...args]);
			expect(opts.cwd).toBe("/workspace");
			if (command === "baton") {
				const value =
					args[1] === "next"
						? {
								status: "running",
								revision: 2,
								attempt: 3,
								step: {
									id: "review",
									human: options.human ?? false,
								},
								agent: { systemPrompt: "review carefully; call report_outcome exactly once" },
								skills: [
									{ id: "assigned", content: "Assigned skill instructions" },
								],
								skillsDirectory: "/workspace/.agents/skills",
								allowedOutcomes: ["approved", "revise_build", "blocked"],
							}
						: {
								task: "Original issue",
								status: "running",
								currentStep: "review",
								session: { revision: options.stale ? 3 : 2 },
								steps: [
									{
										stepId: "human",
										attempt: 1,
										outcome: "responded",
										output: "Human decision",
									},
									{
										stepId: "build",
										attempt: 2,
										outcome: "completed",
										output: "Prior result",
									},
									{ stepId: "review", attempt: 3, output: null },
								],
							};
				return { code: 0, stdout: JSON.stringify(value), stderr: "" };
			}
			throw new Error("Unexpected command");
		},
	};
	const run: typeof runPi = async (command, args, cwd, _signal, progress) => {
		calls.push([command, ...args]);
		expect(cwd).toBe("/workspace");
		const prompt = await readFile((args.at(-1) ?? "").slice(1), "utf8");
		expect(prompt).toContain("Do not invoke baton work, delegate further, or make human decisions.");
		expect(prompt).not.toContain("commit, or push");
		expect(prompt).toContain("Original issue");
		expect(prompt).toContain("review carefully; call report_outcome exactly once");
		const protocolIndex = args.indexOf("--append-system-prompt");
		expect(protocolIndex).toBeGreaterThanOrEqual(0);
		const protocol = args[protocolIndex + 1];
		expect(protocol).toContain("take precedence over conflicting agent instructions");
		expect(protocol).toContain("call report_outcome exactly once");
		expect(args[args.indexOf("--exclude-tools") + 1]).toBe("baton_subagent");
		const extension = await readFile(args[args.indexOf("--extension") + 1], "utf8");
		expect(extension).toContain('name: "report_outcome"');
		const resultPath = extension.match(/const resultFile = (.+);/)?.[1];
		if (!resultPath) throw new Error("Missing outcome file");
		if (options.realPi) {
			const dir = await mkdtemp(join(tmpdir(), "baton-report-probe-"));
			try {
				const probe = join(dir, "probe.mjs");
				const output = join(dir, "tools.json");
				await writeFile(probe, `import { writeFileSync } from "node:fs"; export default function(pi) { pi.on("session_start", () => writeFileSync(${JSON.stringify(output)}, JSON.stringify(pi.getAllTools()))); }`);
				const child = Bun.spawnSync(["pi", "--no-extensions", "--extension", args[args.indexOf("--extension") + 1], "--extension", probe, "--mode", "rpc", "--no-session"], { env: { ...process.env, PI_CODING_AGENT_DIR: dir, PI_OFFLINE: "1" }, stdin: "ignore" });
				expect(child.exitCode).toBe(0);
				const tools = JSON.parse(await readFile(output, "utf8"));
				expect(tools.some((tool: { name: string }) => tool.name === "report_outcome")).toBe(true);
			} finally {
				await rm(dir, { recursive: true, force: true });
			}
		}
		if (options.exerciseTool) {
			const module = await import(args[args.indexOf("--extension") + 1]);
			let report: TestTool | undefined;
			module.default({ registerTool: (tool: TestTool) => { report = tool; } });
			if (!report) throw new Error("Missing report_outcome tool");
			await expect(report.execute("1", { outcome: "invalid", summary: "x" })).rejects.toThrow("Invalid outcome");
			await expect(report.execute("1", { outcome: "approved", summary: " " })).rejects.toThrow("Invalid outcome");
			await expect(report.execute("1", { outcome: "blocked", summary: "Cannot continue", blocker_reason: "", human_action: "Provide access" })).rejects.toThrow("Blocked outcomes require");
			await expect(report.execute("1", { outcome: "blocked", summary: "Cannot continue", blocker_reason: "Missing access", human_action: "" })).rejects.toThrow("Blocked outcomes require");
			const value = { blocker_reason: "", human_action: "", ...JSON.parse(answer) };
			await report.execute("1", value);
			await expect(report.execute("2", value)).rejects.toThrow("already been reported");
		} else {
			await writeFile(JSON.parse(resultPath), answer);
		}
		expect(prompt).toContain("Assigned skill instructions");
		expect(prompt).toContain("/workspace/.agents/skills");
		expect(prompt).toContain("skillsDirectory/<skill-id>/");
		expect(prompt).toContain("Acceptance criteria");
		expect(prompt).toContain("Human decision");
		expect(prompt).toContain("Prior result");
		progress("→ read");
		if (options.code) throw new Error("Subagent failed");
		return answer;
	};
	createExtension(run)(pi as unknown as ExtensionAPI);
	const updates: string[] = [];
	const execute = () => {
		if (!tool) throw new Error("Tool was not registered");
		return tool.execute(
			"1",
			{ runId: "abc", context: "Acceptance criteria" },
			undefined,
			(update: { content: { text: string }[] }) =>
				updates.push(update.content[0].text),
			{
				cwd: "/workspace",
				model: { provider: "test", id: "model" },
				thinkingLevel: "high",
				hasUI: true,
				ui: {
					setStatus: (_key: string, value: string | undefined) =>
						statuses.push(value),
				},
			},
		);
	};
	return { execute, calls, updates, statuses };
}

test("passes current task and instructions to child and returns a submission", async () => {
	const f = fixture(
		JSON.stringify({
			outcome: "approved",
			summary: "Reviewed",
		}),
	);
	const result = await f.execute();
	expect(JSON.parse(result.content[0].text)).toEqual({
		revision: 2,
		stepId: "review",
		attempt: 3,
		outcome: "approved",
		summary: "Reviewed",
	});
	expect(f.calls[2]).toContain("test/model");
	expect(f.calls[2]).toContain("baton_subagent");
	expect(f.calls[2]).toContain("--no-skills");
	expect(f.updates.at(-1)).toContain("Result: approved");
	expect(f.statuses.some((status) => status?.includes("attempt 3"))).toBe(true);
	expect(f.statuses.at(-1)).toBeUndefined();
});

test("report_outcome validates the decision and records it only once", async () => {
	const f = fixture(JSON.stringify({ outcome: "approved", summary: "Reviewed" }), { exerciseTool: true });
	const result = await f.execute();
	expect(JSON.parse(result.content[0].text).outcome).toBe("approved");
});

test("blocked outcome includes the reason and human action in the handoff", async () => {
	const f = fixture(JSON.stringify({ outcome: "blocked", summary: "Cannot continue", blocker_reason: "Missing access", human_action: "Provide access" }), { exerciseTool: true });
	const result = await f.execute();
	expect(JSON.parse(result.content[0].text).summary).toBe("Cannot continue\n\nBlocker: Missing access\nHuman action needed: Provide access");
});

test("real Pi loads the generated report_outcome tool", async () => {
	const f = fixture(JSON.stringify({ outcome: "approved", summary: "Reviewed" }), { realPi: true });
	await f.execute();
});

test("streams child tool progress and validates JSONL completion", async () => {
	const events = [
		{ type: "tool_execution_start", toolName: "read" },
		{ type: "tool_execution_end", toolName: "read", isError: false },
		{
			type: "message_end",
			message: {
				role: "assistant",
				stopReason: "stop",
				content: [{ type: "text", text: "result" }],
			},
		},
		{ type: "agent_settled", aborted: false },
	];
	const progress: string[] = [];
	const script = `for (const x of ${JSON.stringify(events)}) console.log(JSON.stringify(x))`;
	expect(
		await runPi(
			process.execPath,
			["-e", script],
			process.cwd(),
			undefined,
			(x) => progress.push(x),
		),
	).toBe("result");
	expect(progress).toEqual(["→ read", "✓ read"]);
});

test("rejects errored and aborted child processes", async () => {
	const event = {
		type: "message_end",
		message: {
			role: "assistant",
			stopReason: "error",
			errorMessage: "model failed",
			content: [],
		},
	};
	const script = `console.log(JSON.stringify(${JSON.stringify(event)})); console.log(JSON.stringify({type:'agent_settled',aborted:false}))`;
	await expect(
		runPi(process.execPath, ["-e", script], process.cwd(), undefined, () => {}),
	).rejects.toThrow("model failed");
	const controller = new AbortController();
	const pending = runPi(
		process.execPath,
		["-e", "setInterval(() => {}, 1000)"],
		process.cwd(),
		controller.signal,
		() => {},
	);
	controller.abort();
	await expect(pending).rejects.toThrow("aborted");
});

test("rejects malformed and oversized event streams", async () => {
	await expect(
		runPi(
			process.execPath,
			["-e", "console.log('not json')"],
			process.cwd(),
			undefined,
			() => {},
		),
	).rejects.toThrow("Invalid JSON");
	await expect(
		runPi(
			process.execPath,
			["-e", "process.stdout.write('x'.repeat(5 * 1024 * 1024))"],
			process.cwd(),
			undefined,
			() => {},
		),
	).rejects.toThrow("output limit");
});

test("rejects human steps before spawning", async () => {
	const f = fixture("{}", { human: true });
	await expect(f.execute()).rejects.toThrow("not an active agent step");
	expect(f.calls).toHaveLength(1);
});

test("rejects state changes before spawning", async () => {
	const f = fixture("{}", { stale: true });
	await expect(f.execute()).rejects.toThrow("state changed");
	expect(f.calls).toHaveLength(2);
});

test("local Pi package installs the skill and registers the tool", async () => {
	const dir = await mkdtemp(join(tmpdir(), "baton-pi-test-"));
	try {
		const env = { ...process.env, PI_CODING_AGENT_DIR: dir, PI_OFFLINE: "1" };
		const install = Bun.spawnSync(["pi", "install", resolve(".")], { env });
		expect(install.exitCode).toBe(0);
		const probe = join(dir, "probe.ts");
		const output = join(dir, "tools.json");
		await writeFile(
			probe,
			`import { writeFileSync } from "node:fs"; export default function(pi) { pi.on("session_start", () => writeFileSync(${JSON.stringify(output)}, JSON.stringify({ tools: pi.getAllTools().map(t => t.name), commands: pi.getCommands().map(c => c.name) }))); }`,
		);
		const run = Bun.spawnSync(
			["pi", "-e", probe, "--mode", "rpc", "--no-session"],
			{ env, stdin: "ignore" },
		);
		expect(run.exitCode).toBe(0);
		const discovered = JSON.parse(await readFile(output, "utf8"));
		expect(discovered.tools).toContain("baton_subagent");
		expect(discovered.commands).not.toContain("baton-status");
		expect(discovered.commands).toContain("skill:baton");
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

test("rejects failed and invalid child results", async () => {
	await expect(fixture("{}", { code: 1 }).execute()).rejects.toThrow(
		"Subagent failed",
	);
	await expect(fixture("not json").execute()).rejects.toThrow("did not report a valid outcome");
	await expect(
		fixture(
			JSON.stringify({ outcome: "failed", summary: "x" }),
		).execute(),
	).rejects.toThrow("invalid outcome");
});
