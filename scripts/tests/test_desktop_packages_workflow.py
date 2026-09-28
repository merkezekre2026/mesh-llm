from __future__ import annotations

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "desktop-packages.yml"


def job_block(workflow: str, job: str) -> str:
    match = re.search(rf"(?ms)^  {re.escape(job)}:\n(.*?)(?=^  [a-z_]+:\n|\Z)", workflow)
    if match is None:
        raise AssertionError(f"job {job} not found")
    return match.group(1)


class DesktopPackagesWorkflowTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_is_manual_only_with_bounded_inputs(self) -> None:
        on_block = self.workflow.split("\non:\n", 1)[1].split("\nconcurrency:", 1)[0]
        self.assertIn("workflow_dispatch:", on_block)
        for trigger in ("push:", "pull_request", "workflow_run:", "schedule:", "workflow_call:"):
            self.assertNotIn(trigger, on_block)
        self.assertIn("tag:", on_block)
        self.assertIn("attach_to_release:", on_block)
        self.assertIn("type: boolean", on_block)

    def test_permissions_are_read_only_except_the_gated_attach_job(self) -> None:
        top_level = self.workflow.split("\njobs:\n", 1)[0]
        self.assertRegex(top_level, r"(?m)^permissions:\n  contents: read$")
        self.assertEqual(self.workflow.count("contents: write"), 1)
        attach = job_block(self.workflow, "attach")
        self.assertIn("contents: write", attach)
        self.assertIn("inputs.attach_to_release", attach)
        self.assertIn(
            "github.ref == format('refs/heads/{0}', github.event.repository.default_branch)",
            attach,
        )
        self.assertIn("needs: [resolve, package]", attach)

    def test_tag_is_validated_and_never_interpolated_into_shell(self) -> None:
        resolve = job_block(self.workflow, "resolve")
        self.assertIn(r"^v[0-9]+\.[0-9]+\.[0-9]+$", resolve)
        self.assertIn("gh release view", resolve)
        for line in self.workflow.splitlines():
            stripped = line.strip()
            if "${{ inputs." in stripped:
                self.assertTrue(
                    stripped.startswith(("RELEASE_TAG:", "group:", "if:")),
                    f"input interpolated outside env/concurrency/condition: {stripped}",
                )

    def test_packages_verified_release_bytes_instead_of_building_mesh_llm(self) -> None:
        package = job_block(self.workflow, "package")
        self.assertIn("gh release download", package)
        self.assertIn('--pattern "$asset.sha256"', package)
        self.assertIn("scripts/unpack-release-product.py", package)
        self.assertIn("scripts/stage-desktop-sidecar.sh", package)
        self.assertIn("scripts/stage-desktop-sidecar.ps1", package)
        for forbidden in ("release-build", "build-host.sh", "package-native-runtime", "cargo build"):
            self.assertNotIn(forbidden, package)

    def test_covers_macos_and_windows_only_on_hosted_runners(self) -> None:
        package = job_block(self.workflow, "package")
        self.assertIn("os: macos-15", package)
        self.assertIn("os: windows-2022", package)
        self.assertNotIn("ubuntu", package)
        self.assertNotIn("select-ci-runners", self.workflow)
        self.assertNotIn("self-hosted", self.workflow)
        self.assertNotIn("depot-", self.workflow)

    def test_no_ad_hoc_tool_installation(self) -> None:
        for forbidden in (
            "apt-get",
            "apt ",
            "brew install",
            "choco install",
            "cargo install",
            "npm install -g",
            "curl ",
            "pip install",
        ):
            self.assertNotIn(forbidden, self.workflow)
        self.assertIn("npm ci", self.workflow)
        self.assertIn("npm exec -- tauri build", self.workflow)

    def test_sccache_is_initialized_job_locally_before_the_build(self) -> None:
        package = job_block(self.workflow, "package")
        setup = package.index("mozilla-actions/sccache-action@")
        configure = package.index("./.github/actions/configure-sccache-gha")
        build = package.index("npm exec -- tauri build")
        self.assertLess(setup, configure)
        self.assertLess(configure, build)
        self.assertIn('allow_depot_remote_cache: "false"', package)
        self.assertIn('allow_native_github_cache: "false"', package)
        self.assertLess(package.index("MACOSX_DEPLOYMENT_TARGET"), build)
        self.assertIn("scripts/lib/macos-deployment-target.txt", package)

    def test_actions_are_pinned_and_checkouts_drop_credentials(self) -> None:
        uses = re.findall(r"uses: ([^\s]+)", self.workflow)
        self.assertTrue(uses)
        for reference in uses:
            if reference.startswith("./"):
                continue
            self.assertRegex(reference, r"@[0-9a-f]{40}$", reference)
        self.assertEqual(
            self.workflow.count("uses: actions/checkout@"),
            self.workflow.count("persist-credentials: false"),
        )

    def test_installers_get_stable_names_and_checksums(self) -> None:
        package = job_block(self.workflow, "package")
        self.assertIn('stem="mesh-llm-desktop-${VERSION}-${TARGET}"', package)
        self.assertIn(".sha256", package)
        self.assertIn("if-no-files-found: error", package)
        self.assertIn("--config", package)


if __name__ == "__main__":
    unittest.main()
