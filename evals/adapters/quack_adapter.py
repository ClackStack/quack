"""Benchmark adapter registering quack with Harbor and Pier harnesses."""

import json
import os
import shlex
import tempfile
from pathlib import Path, PurePosixPath

from quack_usage import session_usage

BIN = "/usr/local/bin/quack"
DEFAULTS = "/usr/local/share/quack/defaults"
# Both frameworks copy the container's /logs/agent to the trial's agent dir before
# populate_context_post_run, even after a timeout, so the log and the session go there.
AGENT_LOGS = PurePosixPath("/logs/agent")
PROMPT = "/tmp/quack_instruction.txt"
PROVIDERS = {
    "fireworks": ("FIREWORKS_API_KEY", "api.fireworks.ai"),
    "charm-hyper": ("CHARM_API_KEY", "hyper.charm.land"),
    "moonshot": ("MOONSHOT_API_KEY", "api.moonshot.ai"),
    "openrouter": ("OPENROUTER_API_KEY", "openrouter.ai"),
    "together": ("TOGETHER_API_KEY", "api.together.xyz"),
}


def quack_model(name: str | None) -> str:
    m = name or "fireworks/accounts/fireworks/models/kimi-k3"
    if m.startswith("openai/"):
        m = m[len("openai/"):]
    if "/" not in m:
        m = f"fireworks/accounts/fireworks/models/{m}"
    prefix, rest = m.split("/", 1)
    prefix = {"fireworks_ai": "fireworks", "together_ai": "together"}.get(prefix, prefix)
    if prefix not in PROVIDERS:
        raise ValueError(f"Unsupported benchmark provider {prefix!r} · use a configured quack provider")
    m = f"{prefix}/{rest}"
    return m


def api_key(name: str) -> str:
    key = os.environ.get(name) or ""
    if not key and os.path.exists("/etc/environment"):
        try:
            with open("/etc/environment") as ef:
                for line in ef:
                    if line.startswith(name + "="):
                        key = line.split("=", 1)[1].strip().strip('"').strip("'")
                        break
        except OSError:
            pass
    return key


async def install_quack(environment) -> None:
    """Upload the host's binary and defaults bundle, then check the binary runs.

    The binary only ever arrives by upload: Pier inlines install_spec into the agent
    image's Dockerfile, where no binary exists yet. Raising here fails the trial as an
    install error instead of letting it run and score 0 with `quack: command not found`.
    """
    if not os.path.isfile(BIN):
        raise RuntimeError(f"{BIN} is missing on the host · run evals/adapters/install-quack.sh")
    await environment.exec(f"mkdir -p /usr/local/bin {DEFAULTS}", user="root")
    await environment.upload_file(BIN, BIN)
    await environment.upload_dir(DEFAULTS, DEFAULTS)
    await environment.exec(f"chmod +x {BIN}", user="root")
    res = await environment.exec(f"{BIN} --version")
    if res.return_code != 0:
        raise RuntimeError(
            f"quack does not run in the task container (exit {res.return_code}): "
            f"{(res.stdout or '').strip()} {(res.stderr or '').strip()} "
            "· build a static binary with scripts/install.sh --target x86_64-unknown-linux-musl"
        )


async def ensure_quack(environment) -> None:
    res = await environment.exec(f"{BIN} --version")
    if res.return_code != 0:
        await install_quack(environment)


async def upload_prompt(environment, instruction: str) -> None:
    with tempfile.NamedTemporaryFile("w", delete=False) as f:
        f.write(instruction)
        tmp_local = f.name
    try:
        await environment.upload_file(tmp_local, PROMPT)
    finally:
        os.unlink(tmp_local)


async def upload_manifest(environment, model: str) -> None:
    manifest = json.loads((Path(DEFAULTS).parent / "eval-manifest.json").read_text())
    manifest["model"] = quack_model(model)
    manifest["command"] = quack_command(model)
    with tempfile.NamedTemporaryFile("w", delete=False) as handle:
        json.dump(manifest, handle, indent=2)
        local = handle.name
    try:
        await environment.exec(f"mkdir -p {AGENT_LOGS}")
        await environment.upload_file(local, (AGENT_LOGS / "eval-manifest.json").as_posix())
    finally:
        os.unlink(local)


def quack_env(model: str, extra: dict[str, str] | None = None) -> dict[str, str]:
    env = dict(extra or {})
    key_name, _ = PROVIDERS[quack_model(model).split("/", 1)[0]]
    key = env.get(key_name) or api_key(key_name)
    if key:
        env[key_name] = key
    else:
        raise RuntimeError(f"Missing {key_name} · configure the benchmark provider secret")
    env["QUACK_DEFAULTS_DIR"] = DEFAULTS
    env["QUACK_DATA_DIR"] = (AGENT_LOGS / "quack-data").as_posix()
    env["QUACK_CONFIG_DIR"] = (AGENT_LOGS / "quack-config").as_posix()
    env["QUACK_CACHE_DIR"] = (AGENT_LOGS / "quack-cache").as_posix()
    return env


def quack_command(model: str) -> str:
    # Preserve and retain quack's exit status through tee for infrastructure diagnosis.
    log = (AGENT_LOGS / "quack.log").as_posix()
    steps = int(os.environ.get("QUACK_EVAL_MAX_STEPS", "500"))
    if steps <= 0:
        raise ValueError("QUACK_EVAL_MAX_STEPS must be positive")
    args = shlex.join([
        BIN, "--yolo", "--mode", "edit", "--profile", "generic",
        "-c", f"agent.max_steps={steps}", "-c", "tools.edit_format=str_replace",
        "-c", 'model.small=""', "-m", quack_model(model), "exec", "-",
    ])
    return (
        f"set -o pipefail; mkdir -p {AGENT_LOGS}; "
        f"cat {PROMPT} | {args} 2>&1 | tee {log}; "
        f'quack_status=$?; echo "quack exit code: $quack_status" >> {log}; exit "$quack_status"'
    )


def installed_version() -> str:
    manifest = Path(DEFAULTS).parent / "eval-manifest.json"
    return json.loads(manifest.read_text())["version"]


def fill_context(context, logs_dir: Path) -> None:
    usage = session_usage(logs_dir)
    if usage is None:
        return
    fields = getattr(type(context), "model_fields", {})
    for name, value in usage.items():
        if name in fields:
            setattr(context, name, value)


# --- Harbor Adapter ---
try:
    from harbor.agents.installed.base import BaseInstalledAgent as HarborBaseInstalledAgent
    from harbor.environments.base import BaseEnvironment as HarborBaseEnvironment
    from harbor.models.agent.context import AgentContext as HarborAgentContext
    from harbor.agents.model_connection import ModelConnectionSpec as HarborModelConnectionSpec

    class QuackHarborAgent(HarborBaseInstalledAgent):
        MODEL_CONNECTION = HarborModelConnectionSpec(passthrough=True)

        @staticmethod
        def name() -> str:
            return "quack"

        def version(self) -> str | None:
            return installed_version()

        async def install(self, environment: HarborBaseEnvironment) -> None:
            await install_quack(environment)

        def populate_context_post_run(self, context: HarborAgentContext) -> None:
            fill_context(context, self.logs_dir)

        async def run(
            self,
            instruction: str,
            environment: HarborBaseEnvironment,
            context: HarborAgentContext,
        ) -> None:
            await ensure_quack(environment)
            await upload_prompt(environment, instruction)
            await upload_manifest(environment, quack_model(self.model_name))
            env = quack_env(quack_model(self.model_name))
            await environment.exec(quack_command(quack_model(self.model_name)), env=env)
except ImportError:
    pass

# --- Pier Adapter ---
try:
    from pier.agents.installed.base import BaseInstalledAgent as PierBaseInstalledAgent
    from pier.environments.base import BaseEnvironment as PierBaseEnvironment
    from pier.models.agent.context import AgentContext as PierAgentContext
    from pier.models.agent.install import AgentInstallSpec, InstallStep
    from pier.models.agent.network import NetworkAllowlist

    class QuackPierAgent(PierBaseInstalledAgent):
        @staticmethod
        def name() -> str:
            return "quack"

        def version(self) -> str | None:
            return installed_version()

        def network_allowlist(self) -> NetworkAllowlist:
            model = quack_model(self.model_name)
            return NetworkAllowlist(domains=[PROVIDERS[model.split("/", 1)[0]][1]])

        def install_spec(self) -> AgentInstallSpec:
            # Baked into the agent image, where the binary isn't there yet; setup()
            # uploads and checks it.
            return AgentInstallSpec(
                agent_name=self.name(),
                version=self.version(),
                steps=[InstallStep(user="root", run="true")],
            )

        def populate_context_post_run(self, context: PierAgentContext) -> None:
            fill_context(context, self.logs_dir)

        async def setup(self, environment: PierBaseEnvironment) -> None:
            await install_quack(environment)

        async def install(self, environment: PierBaseEnvironment) -> None:
            await install_quack(environment)

        async def run(
            self,
            instruction: str,
            environment: PierBaseEnvironment,
            context: PierAgentContext,
        ) -> None:
            await ensure_quack(environment)
            await upload_prompt(environment, instruction)
            await upload_manifest(environment, quack_model(self.model_name))

            env = environment.agent_process_env({}) or {}
            env = quack_env(quack_model(self.model_name), env)

            # Upload CA certificate if present in the runtime environment
            for ca_cand in [
                "/usr/local/share/ca-certificates/runta-egress.crt",
                "/etc/ssl/certs/ca-certificates.crt",
            ]:
                if os.path.exists(ca_cand):
                    try:
                        await environment.upload_file(ca_cand, "/tmp/runta-ca.crt")
                        env["SSL_CERT_FILE"] = "/tmp/runta-ca.crt"
                        env["REQUESTS_CA_BUNDLE"] = "/tmp/runta-ca.crt"
                        break
                    except Exception:
                        pass

            await environment.exec(quack_command(quack_model(self.model_name)), env=env)
except ImportError:
    pass


def register():
    """Register quack in Harbor and Pier agent factories and enums."""
    try:
        from harbor.models.agent.name import AgentName as HAgentName
        from harbor.agents.factory import AgentFactory as HAgentFactory
        if "quack" not in HAgentName.values():
            m = str.__new__(HAgentName, "quack")
            m._name_ = "QUACK"
            m._value_ = "quack"
            HAgentName._member_map_["QUACK"] = m
            HAgentName._value2member_map_["quack"] = m
            if hasattr(HAgentName, "_member_names_"):
                HAgentName._member_names_.append("QUACK")
            type.__setattr__(HAgentName, "QUACK", m)
        m = HAgentName("quack")
        HAgentFactory._AGENT_MAP[m] = "quack_adapter:QuackHarborAgent"
    except Exception:
        pass

    try:
        from pier.models.agent.name import AgentName as PAgentName
        from pier.agents.factory import AgentFactory as PAgentFactory
        if "quack" not in PAgentName.values():
            m = str.__new__(PAgentName, "quack")
            m._name_ = "QUACK"
            m._value_ = "quack"
            PAgentName._member_map_["QUACK"] = m
            PAgentName._value2member_map_["quack"] = m
            if hasattr(PAgentName, "_member_names_"):
                PAgentName._member_names_.append("QUACK")
            type.__setattr__(PAgentName, "QUACK", m)
        m = PAgentName("quack")
        if "QuackPierAgent" in globals():
            PAgentFactory._AGENT_MAP[m] = QuackPierAgent
    except Exception:
        pass


register()
