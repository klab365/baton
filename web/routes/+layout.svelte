<script lang="ts">
import "../app.css";
import { page } from "$app/state";
import Button from "../components/Button.svelte";
import StatusMessage from "../components/StatusMessage.svelte";

let { data, children } = $props();
const navigation = [
	["/", "Overview"],
	["/agents", "Agents"],
	["/pipelines", "Pipelines"],
	["/tasks", "Tasks"],
	["/settings", "Settings"],
];
</script>

<svelte:head>
	<title>Baton</title>
	<meta name="description" content="Baton orchestrates AI agents." />
</svelte:head>

{#if data.project.status === "missing"}
	<main class="setup">
		<p class="eyebrow">Set up Baton</p>
		<h1>{data.project.name}</h1>
		<p class="lead">This project does not have a Baton configuration yet.</p>
		<dl><dt>Project directory</dt><dd>{data.project.directory}</dd></dl>
		<p class="muted">Setup creates a minimal versioned configuration at <code>.baton/config.json</code>.</p>
		<form method="POST" action="/?/initialize"><Button type="submit">Set up project</Button></form>
	</main>
{:else if data.project.status === "invalid"}
	<main class="setup">
		<p class="eyebrow">Configuration error</p>
		<h1>Could not open this Baton project</h1>
		<StatusMessage tone="error" title="Configuration needs attention">{data.project.message}</StatusMessage>
		<p class="muted">The existing configuration has not been changed.</p>
	</main>
{:else}
	<div class="shell">
		<header class="app-header">
			<a class="brand" href="/">Baton</a><span class="project-name">{data.project.name}</span>
		</header>
		<nav aria-label="Main navigation">
			{#each navigation as [href, label]}
				<a href={href} aria-current={page.url.pathname === href ? "page" : undefined}>{label}</a>
			{/each}
		</nav>
		<main id="main-content">{@render children()}</main>
	</div>
{/if}
