"""Offline checks for module detection. Run: python3 -m unittest discover -s tests"""
import json, shutil, sys, tempfile, unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from moru_narrow import kotlin, typescript  # noqa: E402

rg = shutil.which("rg")


def write(root, files):
    for rel, text in files.items():
        p = Path(root) / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)


@unittest.skipUnless(rg, "ripgrep is required")
class KotlinModules(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp)

    def tier1(self, name, decl):
        res = kotlin.recommend(kotlin.Repo(self.tmp), name, {decl})
        return set(res["tiers"][1])

    def project(self, dep_line, extra=None):
        files = {
            "settings.gradle.kts": 'include(":core:ui")\ninclude(":feature:home")\n',
            "core/ui/build.gradle.kts": "",
            "core/ui/src/main/kotlin/Card.kt": "package app.ui\nfun Card(title: String) {}\n",
            "feature/home/build.gradle.kts": dep_line,
            "feature/home/src/main/kotlin/Home.kt": "package app.home\nimport app.ui.Card\nfun home() { Card(\"x\") }\n",
        }
        files.update(extra or {})
        write(self.tmp, files)

    def test_project_string_dependency(self):
        self.project('dependencies { implementation(project(":core:ui")) }')
        self.assertIn("feature/home/src/main/kotlin/Home.kt", self.tier1("Card", "core/ui/src/main/kotlin/Card.kt"))

    def test_project_path_argument(self):
        self.project('dependencies { implementation(project(path = ":core:ui")) }')
        self.assertIn("feature/home/src/main/kotlin/Home.kt", self.tier1("Card", "core/ui/src/main/kotlin/Card.kt"))

    def test_typesafe_accessor(self):
        self.project("dependencies { implementation(projects.core.ui) }")
        self.assertIn("feature/home/src/main/kotlin/Home.kt", self.tier1("Card", "core/ui/src/main/kotlin/Card.kt"))

    def test_convention_plugin_dependency(self):
        self.project("plugins { id(\"app.feature\") }", {
            "build-logic/convention/src/main/kotlin/FeaturePlugin.kt": 'fun Project.deps() { add("implementation", project(":core:ui")) }\n',
        })
        self.assertIn("feature/home/src/main/kotlin/Home.kt", self.tier1("Card", "core/ui/src/main/kotlin/Card.kt"))

    def test_registered_convention_plugin_applies_only_to_its_modules(self):
        self.project('plugins { alias(libs.plugins.app.feature) }', {
            "gradle/libs.versions.toml": '[plugins]\napp-feature = { id = "app.feature" }\n',
            "build-logic/convention/build.gradle.kts": 'register("feature") {\n id = libs.plugins.app.feature.get().pluginId\n implementationClass = "FeaturePlugin"\n}\n',
            "build-logic/convention/src/main/kotlin/FeaturePlugin.kt": 'class FeaturePlugin { fun f() { add("implementation", project(":core:ui")) } }\n',
            "feature/other/build.gradle.kts": "",
            "feature/other/src/main/kotlin/Other.kt": "package app.other\nimport app.ui.Card\nfun o() { Card(\"x\") }\n",
        })
        t1 = self.tier1("Card", "core/ui/src/main/kotlin/Card.kt")
        self.assertIn("feature/home/src/main/kotlin/Home.kt", t1)
        self.assertNotIn("feature/other/src/main/kotlin/Other.kt", t1)

    def test_declarations_in_two_files_are_grouped(self):
        write(self.tmp, {
            "settings.gradle.kts": 'include(":a")\ninclude(":b")\ninclude(":app")\n',
            "a/build.gradle.kts": "", "b/build.gradle.kts": "",
            "app/build.gradle.kts": 'dependencies { implementation(project(":a")) }',
            "a/src/main/kotlin/A.kt": "package pa\nfun icon(x: Int) {}\n",
            "b/src/main/kotlin/B.kt": "package pb\nfun icon(x: Int) {}\n",
            "app/src/main/kotlin/Use.kt": "package pu\nimport pa.icon\nfun u() { icon(1) }\n",
        })
        res = kotlin.recommend(kotlin.Repo(self.tmp), "icon", None)
        self.assertEqual(res["groups"]["a/src/main/kotlin/A.kt"], ["app/src/main/kotlin/Use.kt"])
        self.assertEqual(res["groups"]["b/src/main/kotlin/B.kt"], [])
        self.assertIn("sees the declaration in a/src/main/kotlin/A.kt", kotlin.render(res, False))

    def test_unrelated_module_stays_demoted(self):
        self.project("")
        self.assertNotIn("feature/home/src/main/kotlin/Home.kt", self.tier1("Card", "core/ui/src/main/kotlin/Card.kt"))


@unittest.skipUnless(rg, "ripgrep is required")
class TypeScriptResolution(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp)

    def tier1(self, name, decl):
        res = typescript.recommend(typescript.Repo(self.tmp), name, {decl})
        return set(res["tiers"][1])

    def test_workspace_package_exports(self):
        write(self.tmp, {
            "packages/ui/package.json": json.dumps({"name": "@acme/ui", "exports": {"./button": "./src/button.tsx"}}),
            "packages/ui/src/button.tsx": "export function Button(p: {a: string}) { return null }\n",
            "apps/web/package.json": json.dumps({"name": "web"}),
            "apps/web/page.tsx": 'import { Button } from "@acme/ui/button";\nexport const x = <Button a="1" />;\n',
        })
        self.assertIn("apps/web/page.tsx", self.tier1("Button", "packages/ui/src/button.tsx"))

    def test_tsconfig_extends_paths(self):
        write(self.tmp, {
            "tooling/base.json": json.dumps({"compilerOptions": {"baseUrl": ".", "paths": {"~/*": ["../app/src/*"]}}}),
            "app/tsconfig.json": json.dumps({"extends": "../tooling/base.json"}),
            "app/src/lib.ts": "export function load(id: string) { return id }\n",
            "app/src/use.ts": 'import { load } from "~/lib";\nload("1");\n',
        })
        self.assertIn("app/src/use.ts", self.tier1("load", "app/src/lib.ts"))

    def test_flow_lists_same_folder_importers_without_the_name(self):
        write(self.tmp, {
            "tsconfig.json": "{}",
            "src/lib.ts": "export function save(id: string) { return id }\n",
            "src/svc/store.ts": 'import { save } from "../lib";\nexport const put = () => save("1");\n',
            "src/svc/runtime.ts": 'import { put } from "./store";\nexport const run = () => put();\n',
        })
        res = typescript.recommend(typescript.Repo(self.tmp), "save", {"src/lib.ts"})
        self.assertEqual(res["flow"], ["src/svc/runtime.ts"])
        self.assertIn("src/svc/runtime.ts", typescript.render(res, False, True))
        self.assertNotIn("src/svc/runtime.ts", typescript.render(res, False, False))

    def test_js_extension_specifier(self):
        write(self.tmp, {
            "tsconfig.json": "{}",
            "src/lib.ts": "export function load(id: string) { return id }\n",
            "src/use.ts": 'import { load } from "./lib.js";\nload("1");\n',
        })
        self.assertIn("src/use.ts", self.tier1("load", "src/lib.ts"))


if __name__ == "__main__":
    unittest.main()
