import { expect, test } from "@playwright/test";

test("switching a populated Rig agent to Pi clears Rig selections and round-trips Pi model", async ({
	page,
}) => {
	const existing = {
		id: "agent-one",
		instructions: "test role",
		provider: "rig",
		model: "openai/gpt-4.1",
		credentialId: "rig-key",
		skillIds: [],
		allowedTools: [],
	};
	let saved: { agent?: typeof existing } | undefined;
	let currentAgent = existing;
	await page.route("**/api/**", async (route) => {
		const path = new URL(route.request().url()).pathname;
		if (path === "/api/project")
			await route.fulfill({
				json: { status: "ready", name: "Test", config: {} },
			});
		else if (path === "/api/agents" && route.request().method() === "GET")
			await route.fulfill({
				json: { revision: "rev", agents: [currentAgent], skills: [] },
			});
		else if (
			path === "/api/agents/agent-one" &&
			route.request().method() === "PUT"
		) {
			saved = route.request().postDataJSON();
			currentAgent = (saved as { agent?: typeof existing }).agent ?? existing;
			await route.fulfill({ json: {} });
		} else if (path === "/api/pi/models/custom-provider")
			await route.fulfill({
				json: [{ id: "catalog/model-v1", label: "Catalog model" }],
			});
		else if (path.startsWith("/api/pi/models/"))
			await route.fulfill({ json: [] });
		else if (path === "/api/credentials/rig-key/models")
			await route.fulfill({ json: [{ id: "gpt-4.1", label: "GPT-4.1" }] });
		else if (path === "/api/credentials")
			await route.fulfill({
				json: [
					{
						id: "rig-key",
						name: "Rig key",
						credentialType: "openai",
						status: "available",
					},
				],
			});
		else if (path === "/api/credential-types")
			await route.fulfill({
				json: [{ id: "openai", label: "OpenAI", provider: "openai" }],
			});
		else await route.fulfill({ json: {} });
	});
	await page.goto("/agents");
	await page.getByRole("button", { name: /agent-one/ }).click();
	await page.getByLabel("Runtime").click();
	await page.getByRole("option", { name: "Pi CLI" }).click();
	await expect(
		page.getByRole("combobox", { name: "Pi provider" }),
	).toContainText("Select Pi provider");
	await expect(page.getByLabel("Pi model ID (manual)")).toHaveValue("");
	await expect(
		page.getByText(/pi --offline --list-models PROVIDER/),
	).toBeVisible();
	await page.getByRole("combobox", { name: "Pi provider" }).click();
	await page.getByRole("option", { name: "Other / custom provider" }).click();
	await page.getByLabel("Custom Pi provider ID").fill("custom-provider");
	await page.locator("#pi-model-picker").click();
	await page.getByRole("option", { name: "Catalog model" }).click();
	await expect(page.getByLabel("Pi model ID (manual)")).toHaveValue(
		"catalog/model-v1",
	);
	await page.getByLabel("Pi model ID (manual)").fill("");
	await page.getByLabel("Custom Pi provider ID").fill("");
	await page.getByLabel("Pi model ID (manual)").fill("legacy-model");
	await page.getByRole("button", { name: "Save agent" }).click();
	await expect(page.getByRole("alert")).toContainText("Choose a provider");
	await page.getByLabel("Custom Pi provider ID").fill("custom/provider");
	await page.getByLabel("Pi model ID (manual)").fill("model/with/slash");
	await page.getByRole("button", { name: "Save agent" }).click();
	await expect(page.getByRole("alert")).toContainText("cannot contain '/'");
	await page.getByLabel("Custom Pi provider ID").fill("custom-provider");
	await page.getByLabel("Pi model ID (manual)").fill("model/with/slash");
	await page.getByRole("button", { name: "Save agent" }).click();
	await expect(page.getByRole("status")).toContainText("Agent saved");
	await expect.poll(() => saved?.agent?.credentialId).toBeNull();
	await expect
		.poll(() => saved?.agent?.model)
		.toBe("custom-provider/model/with/slash");
	await page.getByRole("button", { name: /agent-one/ }).click();
	await expect(page.getByLabel("Custom Pi provider ID")).toHaveValue(
		"custom-provider",
	);
	await expect(page.getByLabel("Pi model ID (manual)")).toHaveValue(
		"model/with/slash",
	);
});

test("changing Pi providers ignores a stale model-list response", async ({
	page,
}) => {
	await page.route("**/api/**", async (route) => {
		const path = new URL(route.request().url()).pathname;
		if (path === "/api/project")
			await route.fulfill({
				json: { status: "ready", name: "Test", config: {} },
			});
		else if (path === "/api/agents")
			await route.fulfill({
				json: { revision: "rev", agents: [], skills: [] },
			});
		else if (path === "/api/credentials" || path === "/api/credential-types")
			await route.fulfill({ json: [] });
		else if (path === "/api/pi/models/anthropic") {
			await new Promise((resolve) => setTimeout(resolve, 350));
			await route.fulfill({
				json: [{ id: "stale-model", label: "Stale model" }],
			});
		} else if (path === "/api/pi/models/amazon-bedrock")
			await route.fulfill({
				json: [{ id: "bedrock-model", label: "Bedrock model" }],
			});
		else await route.fulfill({ json: [] });
	});
	await page.goto("/agents");
	await page.getByRole("button", { name: "Create agent" }).click();
	await page.getByLabel("Runtime").click();
	await page.getByRole("option", { name: "Pi CLI" }).click();
	await page.getByRole("combobox", { name: "Pi provider" }).click();
	await page.getByRole("option", { name: "Anthropic" }).click();
	await page.getByRole("combobox", { name: "Pi provider" }).click();
	await page.getByRole("option", { name: "Amazon Bedrock" }).click();
	await expect(page.locator("#pi-model-picker")).toBeEnabled();
	await page.locator("#pi-model-picker").click();
	await expect(
		page.getByRole("option", { name: "Bedrock model" }),
	).toBeVisible();
	await expect(page.getByRole("option", { name: "Stale model" })).toHaveCount(
		0,
	);
	await page.waitForTimeout(400);
	await expect(page.getByRole("option", { name: "Stale model" })).toHaveCount(
		0,
	);
});

test("unavailable Pi model catalog leaves manual entry usable", async ({
	page,
}) => {
	let saved: { agent?: { model?: string | null } } | undefined;
	await page.route("**/api/**", async (route) => {
		const path = new URL(route.request().url()).pathname;
		if (path === "/api/project")
			await route.fulfill({
				json: { status: "ready", name: "Test", config: {} },
			});
		else if (path === "/api/agents" && route.request().method() === "GET")
			await route.fulfill({
				json: { revision: "rev", agents: [], skills: [] },
			});
		else if (path === "/api/agents" && route.request().method() === "POST") {
			saved = route.request().postDataJSON();
			await route.fulfill({ json: {} });
		} else if (path === "/api/credentials" || path === "/api/credential-types")
			await route.fulfill({ json: [] });
		else if (path === "/api/pi/models/anthropic")
			await route.fulfill({
				status: 503,
				json: { error: "Pi CLI unavailable" },
			});
		else await route.fulfill({ json: [] });
	});
	await page.goto("/agents");
	await page.getByRole("button", { name: "Create agent" }).click();
	await page.getByLabel("Runtime").click();
	await page.getByRole("option", { name: "Pi CLI" }).click();
	await page.getByLabel("Agent ID").fill("pi-new");
	await page.getByLabel("Instructions").fill("test role");
	await page.getByRole("combobox", { name: "Pi provider" }).click();
	await page.getByRole("option", { name: "Anthropic" }).click();
	await expect(page.getByRole("alert")).toContainText(
		"enter the model ID manually",
	);
	await page.getByLabel("Pi model ID (manual)").fill("claude-custom/model");
	await page.getByRole("button", { name: "Save agent" }).click();
	await expect(page.getByRole("status")).toContainText("Agent saved");
	await expect
		.poll(() => saved?.agent?.model)
		.toBe("anthropic/claude-custom/model");
});

test("empty Pi catalog keeps manual entry available and requires explicit Pi default selection", async ({
	page,
}) => {
	let saved: { agent?: { model?: string | null } } | undefined;
	await page.route("**/api/**", async (route) => {
		const path = new URL(route.request().url()).pathname;
		if (path === "/api/project")
			await route.fulfill({
				json: { status: "ready", name: "Test", config: {} },
			});
		else if (path === "/api/agents" && route.request().method() === "GET")
			await route.fulfill({
				json: { revision: "rev", agents: [], skills: [] },
			});
		else if (path === "/api/agents" && route.request().method() === "POST") {
			saved = route.request().postDataJSON();
			await route.fulfill({ json: {} });
		} else if (path === "/api/credentials" || path === "/api/credential-types")
			await route.fulfill({ json: [] });
		else if (path === "/api/pi/models/anthropic")
			await route.fulfill({ json: [] });
		else await route.fulfill({ json: [] });
	});
	await page.goto("/agents");
	await page.getByRole("button", { name: "Create agent" }).click();
	await page.getByLabel("Agent ID").fill("pi-default");
	await page.getByLabel("Instructions").fill("test role");
	await page.getByLabel("Runtime").click();
	await page.getByRole("option", { name: "Pi CLI" }).click();
	await page.getByRole("combobox", { name: "Pi provider" }).click();
	await page.getByRole("option", { name: "Anthropic" }).click();
	await expect(page.getByRole("status")).toContainText("No models were listed");
	await page.getByLabel("Pi model ID (manual)").fill("custom-model");
	await expect(page.getByLabel("Pi model ID (manual)")).toHaveValue(
		"custom-model",
	);
	await page.getByLabel("Pi model ID (manual)").fill("");
	await page.getByRole("button", { name: "Save agent" }).click();
	await expect(page.getByRole("alert")).toContainText(
		"Use Pi configured default",
	);
	await page.getByRole("combobox", { name: "Pi provider" }).click();
	await page.getByRole("option", { name: "Use Pi configured default" }).click();
	await page.getByRole("button", { name: "Save agent" }).click();
	await expect(page.getByRole("status")).toContainText("Agent saved");
	await expect.poll(() => saved?.agent?.model).toBeNull();
});

test("changing Pi provider clears the old model before save and round-trips the new model", async ({
	page,
}) => {
	let currentAgent: Record<string, unknown> | undefined;
	let saved: { agent?: { model?: string | null } } | undefined;
	await page.route("**/api/**", async (route) => {
		const path = new URL(route.request().url()).pathname;
		if (path === "/api/project")
			await route.fulfill({
				json: { status: "ready", name: "Test", config: {} },
			});
		else if (path === "/api/agents" && route.request().method() === "GET")
			await route.fulfill({
				json: {
					revision: "rev",
					agents: currentAgent ? [currentAgent] : [],
					skills: [],
				},
			});
		else if (path === "/api/agents" && route.request().method() === "POST") {
			saved = route.request().postDataJSON();
			currentAgent = saved?.agent;
			await route.fulfill({ json: {} });
		} else if (path === "/api/credentials" || path === "/api/credential-types")
			await route.fulfill({ json: [] });
		else if (path === "/api/pi/models/anthropic")
			await route.fulfill({
				json: [{ id: "claude-model", label: "Claude model" }],
			});
		else if (path === "/api/pi/models/amazon-bedrock")
			await route.fulfill({
				json: [{ id: "nova-model", label: "Nova model" }],
			});
		else await route.fulfill({ json: [] });
	});
	await page.goto("/agents");
	await page.getByRole("button", { name: "Create agent" }).click();
	await page.getByLabel("Agent ID").fill("pi-provider-switch");
	await page.getByLabel("Instructions").fill("test role");
	await page.getByLabel("Runtime").click();
	await page.getByRole("option", { name: "Pi CLI" }).click();
	await page.getByRole("combobox", { name: "Pi provider" }).click();
	await page.getByRole("option", { name: "Anthropic" }).click();
	await page.locator("#pi-model-picker").click();
	await page.getByRole("option", { name: "Claude model" }).click();
	await page.getByRole("combobox", { name: "Pi provider" }).click();
	await page.getByRole("option", { name: "Other / custom provider" }).click();
	await page.getByLabel("Custom Pi provider ID").fill("custom-one");
	await page.getByLabel("Pi model ID (manual)").fill("custom-model");
	await page.getByLabel("Custom Pi provider ID").fill("custom-two");
	await expect(page.getByLabel("Pi model ID (manual)")).toHaveValue("");
	await page.getByRole("combobox", { name: "Pi provider" }).click();
	await page.getByRole("option", { name: "Amazon Bedrock" }).click();
	await expect(page.getByLabel("Pi model ID (manual)")).toHaveValue("");
	await page.locator("#pi-model-picker").click();
	await page.getByRole("option", { name: "Nova model" }).click();
	await page.getByRole("button", { name: "Save agent" }).click();
	await expect(page.getByRole("status")).toContainText("Agent saved");
	await expect
		.poll(() => saved?.agent?.model)
		.toBe("amazon-bedrock/nova-model");
	await page.getByRole("button", { name: /pi-provider-switch/ }).click();
	await expect(page.getByLabel("Pi model ID (manual)")).toHaveValue(
		"nova-model",
	);
});
