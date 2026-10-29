<script lang="ts">
import "../app.css";
import { page } from "$app/state";
import {
	Activity,
	Boxes,
	CheckSquare,
	GitBranch,
	History,
	Settings2,
	Workflow,
	Sparkles,
} from "@lucide/svelte";
import Button from "../components/Button.svelte";
import StatusMessage from "../components/StatusMessage.svelte";

let { data, children } = $props();
const navigation = [
	{ href: "/", label: "Overview", icon: Activity },
	{ href: "/tasks", label: "Task board", icon: CheckSquare },
	{ href: "/runs", label: "Runs", icon: History },
	{ href: "/agents", label: "Agents", icon: Boxes },
	{ href: "/pipelines", label: "Pipelines", icon: Workflow },
	{ href: "/skills", label: "Skills", icon: Sparkles },
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
	<div class="app-shell">
		<aside class="sidebar" aria-label="Main navigation">
			<a class="brand" href="/" aria-label="Baton overview"><span class="brand-mark"><GitBranch size={19} /></span><span>Baton</span></a>
			<div class="project-block"><span class="project-dot"></span><span class="project-label">PROJECT</span><strong title={data.project.name}>{data.project.name}</strong></div>
			<nav class="primary-nav">
				{#each navigation as item}
					<a href={item.href} aria-current={page.url.pathname === item.href || (item.href !== "/" && page.url.pathname.startsWith(`${item.href}/`)) ? "page" : undefined}>
						<item.icon size={17} strokeWidth={1.8} /><span>{item.label}</span>
					</a>
				{/each}
			</nav>
			<div class="sidebar-bottom"><a href="/settings" aria-current={page.url.pathname.startsWith("/settings") ? "page" : undefined}><Settings2 size={17} /><span>Settings</span></a><div class="sidebar-footnote">AI workflow orchestration</div></div>
		</aside>
		<div class="workspace">
			<header class="mobile-header"><a class="brand" href="/"><span class="brand-mark"><GitBranch size={18} /></span>Baton</a><span class="mobile-project-name" title={data.project.name}>{data.project.name}</span></header>
			<main id="main-content" class="page-content">{@render children()}</main>
		</div>
	</div>
{/if}
