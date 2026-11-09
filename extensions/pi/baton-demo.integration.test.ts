import { expect, test } from "bun:test";
import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { createExtension } from "./baton-subagent";

async function runDemo(live: boolean) {
	const root = resolve(".");
	const dir = await mkdtemp(join(tmpdir(), "baton-pi-demo-"));
	const workspace = join(dir, "workspace");
	const agentDir = join(dir, "pi");
	const previous = {
		agent: process.env.PI_CODING_AGENT_DIR,
		offline: process.env.PI_OFFLINE,
	};
	let requests = 0;
	const server = Bun.serve({
		port: 0,
		fetch: async (request) => {
			const body = (await request.json()) as { messages: { role: string }[] };
			const isFinal = body.messages.some((message) => message.role === "tool");
			requests++;
			const delta = isFinal
				? { content: "Plan complete." }
				: {
						tool_calls: [
							{
								index: 0,
								id: "call_1",
								type: "function",
								function: {
									name: "report_outcome",
									arguments: JSON.stringify({
										outcome: "completed",
										summary: "Plan ready for implementation.",
									}),
								},
							},
						],
					};
			const event = {
				id: "test",
				object: "chat.completion.chunk",
				created: 1,
				model: "fake",
				choices: [
					{ index: 0, delta, finish_reason: isFinal ? "stop" : "tool_calls" },
				],
			};
			return new Response(
				`data: ${JSON.stringify(event)}\n\ndata: [DONE]\n\n`,
				{ headers: { "content-type": "text/event-stream" } },
			);
		},
	});
	try {
		await mkdir(join(workspace, ".baton"), { recursive: true });
		await mkdir(agentDir, { recursive: true });
		expect(Bun.spawnSync(["git", "init", "-q", workspace]).exitCode).toBe(0);
		await cp(
			join(root, "examples/demo/.baton/config.json"),
			join(workspace, ".baton/config.json"),
		);
		await cp(join(root, ".agents/skills"), join(workspace, ".agents/skills"), {
			recursive: true,
		});
		await writeFile(
			join(agentDir, "models.json"),
			JSON.stringify({
				providers: {
					test: {
						baseUrl: live
							? process.env.OLLAMA_BASE_URL || "https://ollama.com/v1"
							: `http://127.0.0.1:${server.port}/v1`,
						api: "openai-completions",
						apiKey: live ? "$OLLAMA_API_KEY" : "test",
						models: [{ id: live ? process.env.OLLAMA_MODEL : "fake" }],
					},
				},
			}),
		);
		process.env.PI_CODING_AGENT_DIR = agentDir;
		process.env.PI_OFFLINE = "1";
		const build = Bun.spawnSync(
			["cargo", "build", "-q", "-p", "baton-cli", "--bin", "baton"],
			{ cwd: root },
		);
		expect(build.exitCode).toBe(0);
		const baton = join(root, "target/debug/baton");
		async function command(args: string[]) {
			const child = Bun.spawn([baton, "work", ...args], {
				cwd: workspace,
				stdout: "pipe",
				stderr: "pipe",
			});
			const stdout = await new Response(child.stdout).text();
			const stderr = await new Response(child.stderr).text();
			const code = await child.exited;
			if (code !== 0) throw new Error(stderr);
			return JSON.parse(stdout);
		}
		const start = await command([
			"start",
			"--issue",
			"Plan adding a short greeting to a README. No implementation is required in this planning step.",
		]);
		let tool:
			| {
					execute: (
						...args: unknown[]
					) => Promise<{ content: { text: string }[] }>;
			  }
			| undefined;
		const pi = {
			registerTool(value: typeof tool) {
				tool = value;
			},
			async exec(_command: string, args: string[]) {
				try {
					return {
						code: 0,
						stdout: JSON.stringify(await command(args.slice(1))),
						stderr: "",
					};
				} catch (error) {
					return { code: 1, stdout: "", stderr: String(error) };
				}
			},
		};
		createExtension()(pi as unknown as ExtensionAPI);
		if (!tool) throw new Error("Tool not registered");
		const result = await tool.execute(
			"1",
			{ runId: start.id, context: "Test acceptance criteria" },
			undefined,
			undefined,
			{
				cwd: workspace,
				model: {
					provider: "test",
					id: live ? process.env.OLLAMA_MODEL : "fake",
				},
				thinkingLevel: "off",
				hasUI: false,
			},
		);
		const submission = JSON.parse(result.content[0].text);
		expect(submission).toMatchObject({ stepId: "plan", outcome: "completed" });
		expect(typeof submission.summary).toBe("string");
		expect(submission.summary.trim()).not.toBe("");
		if (!live) {
			expect(submission.summary).toBe("Plan ready for implementation.");
			expect(requests).toBeGreaterThanOrEqual(2);
		}
		const resultFile = join(dir, "result.json");
		await writeFile(resultFile, JSON.stringify(submission));
		const next = await command([
			"submit",
			"--run",
			start.id,
			"--result",
			resultFile,
		]);
		expect(next.currentStep).toBe("build");
		expect(
			await readFile(join(workspace, ".baton/config.json"), "utf8"),
		).toContain("call report_outcome exactly once");
	} finally {
		server.stop(true);
		if (previous.agent === undefined) delete process.env.PI_CODING_AGENT_DIR;
		else process.env.PI_CODING_AGENT_DIR = previous.agent;
		if (previous.offline === undefined) delete process.env.PI_OFFLINE;
		else process.env.PI_OFFLINE = previous.offline;
		await rm(dir, { recursive: true, force: true });
	}
}

test(
	"demo planner reports an outcome through Pi and Baton",
	() => runDemo(false),
	120_000,
);

test.skipIf(process.env.OLLAMA_LIVE_TEST !== "1")(
	"demo planner reports an outcome using Ollama Cloud",
	async () => {
		if (!process.env.OLLAMA_API_KEY || !process.env.OLLAMA_MODEL) {
			throw new Error("Set OLLAMA_API_KEY and OLLAMA_MODEL for the live test");
		}
		await runDemo(true);
	},
	300_000,
);
