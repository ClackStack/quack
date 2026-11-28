<script lang="ts">
	import Plate from '#lib/components/Plate.svelte';
	import PlateHeading from '#lib/components/PlateHeading.svelte';

	const points = [
		['No hardcoded external endpoints', 'Every provider and MCP server lives in a small TOML file (42 provider files ship as data). Delete a file and that integration is gone; automated tests verify startup stays clean with any default file removed. Only the core prompt and sub-agents are compiled in.'],
		['Hosts, redirects & failover', '`[policy.providers] hosts` validates every URL in `base_url` and multi-node `base_urls`, `[policy.mcp] hosts` restricts remote MCP and OAuth endpoints, cross-host HTTP redirects must pass the egress gate, opt-in `web_fetch` is off by default and disabled offline, and the Session Egress Ledger logs every hop and block.'],
		['Policy locks & Windows Registry', 'System-layer locks hold against user files, `.quack/`, env, CLI, `/config`, and `Ctrl-L` writes — including `[network] secret_guard`. On Windows, quack reads `HKLM\\SOFTWARE\\Policies\\quack` (`SystemDir` and `PolicyToml`) directly so GPO, MDM, and Intune policies cannot be redirected.'],
		['Audit log & session retention', 'Lockable `[audit]` appends mode-`0600` JSONL records (`ts`, `session_id`, `tool`, `subject`, `decision`, `decided_by`, `ok`, `duration_ms`) with arguments omitted by default; `[sessions] retention_days` and `quack sessions prune` manage transcript retention, verified by `quack doctor`.'],
		['Hooks users can’t remove', 'Hooks in the system config directory are enforced across seven events (including guided `forge-review` and `jira` hooks with masked credentials and live draft checks) in any language.'],
		['Plucked installs (--plucked)', 'Pass `--plucked` (`-Plucked`) to install the full agent without any preinstalled hosted-service providers or `defaults/config.toml`, ready for teams running internal model servers. Updates preserve plucked installs automatically.'],
		['Windows MSI & verified releases', 'Tagged releases publish GPO/MDM-ready WiX v5 `.msi` packages (`ALLUSERS=2`, MST-ready `DefaultsFeature`), `.deb`/`.rpm` packages, CycloneDX 1.5 SBOMs, `SHA256SUMS`, and keyless GitHub SLSA build provenance attestations (`gh attestation verify`).'],
		['Apache-2.0 with minimal dependencies', 'Every dependency is audited per OS before being added. TLS uses `ring`, and Linux keyring support calls `secret-tool` directly instead of pulling in a D-Bus stack.']
	];
	const parts = (t: string) => t.split(/(`[^`]+`)/).filter(Boolean);
</script>

<Plate id="organisations" color="var(--color-duck)">
		<PlateHeading title="For teams rolling it out">
			The same coding agent, with the policy and audit controls security teams need to approve it.
		</PlateHeading>

		<div class="grid grid-cols-1 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)] gap-12 items-start">
			<dl class="grid sm:grid-cols-2 gap-x-10 gap-y-8">
				{#each points as [term, text] (term)}
					<div class="border-t-2 border-ink pt-4">
						<dt class="font-display text-xl font-extrabold">{term}</dt>
						<dd class="mt-2 text-ink-2 leading-relaxed">
							{#each parts(text) as p, i (i)}{#if p.startsWith('`')}<code class="text-[0.9em] text-ink">{p.slice(1, -1)}</code>{:else}{p}{/if}{/each}
						</dd>
					</div>
				{/each}
			</dl>

			<figure class="lg:sticky lg:top-8 space-y-4">
				<div class="rounded-xl border-2 border-ink bg-term text-term-text overflow-hidden">
					<p class="px-4 py-2 text-[13px] font-mono text-term-dim border-b border-white/10">providers.d/lab.toml</p>
					<pre class="px-4 py-4 text-[13px] sm:text-[14px] leading-relaxed overflow-x-auto"><span class="text-sky">base_url</span> = <span class="text-duck">"http://gpu-01.lab:8000/v1"</span></pre>
				</div>
				<div class="rounded-xl border-2 border-ink bg-term text-term-text overflow-hidden">
					<p class="px-4 py-2 text-[13px] font-mono text-term-dim border-b border-white/10">/etc/quack/config.toml</p>
					<pre class="px-4 py-4 text-[13px] sm:text-[14px] leading-relaxed overflow-x-auto"><span class="text-term-dim">[policy.providers]</span>
<span class="text-sky">hosts</span> = [<span class="text-duck">"gpu-01.lab"</span>]</pre>
				</div>
				<figcaption class="text-sm text-ink-2">
					A provider needs only its URL; models are discovered. The policy keeps every provider to the lab’s own
					server.
				</figcaption>
			</figure>
		</div>
	</Plate>
