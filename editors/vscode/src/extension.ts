import * as vscode from "vscode";
import * as fs from "fs";
import * as path from "path";
import { runWasm, WasiResult } from "./wasi-shim";

let diagnosticCollection: vscode.DiagnosticCollection;

/** The compiled WASM module, loaded once on activation. */
let wasmModule: BufferSource | undefined;

export function activate(context: vscode.ExtensionContext) {
  diagnosticCollection =
    vscode.languages.createDiagnosticCollection("hdlfmtr");
  context.subscriptions.push(diagnosticCollection);

  // Pre-load the WASM binary from the extension directory.
  const wasmPath = path.join(context.extensionPath, "wasm", "hdlfmtr.wasm");
  try {
    wasmModule = fs.readFileSync(wasmPath);
  } catch {
    vscode.window.showErrorMessage(
      `hdlfmtr: could not load ${wasmPath}. The extension may be installed incorrectly.`
    );
  }

  // Register as the document formatter for VHDL files.
  const formatter =
    vscode.languages.registerDocumentFormattingEditProvider("vhdl", {
      provideDocumentFormattingEdits(
        document: vscode.TextDocument
      ): Thenable<vscode.TextEdit[]> {
        return formatDocument(document);
      },
    });
  context.subscriptions.push(formatter);

  // Manual command: "hdlfmtr: Format Document"
  const command = vscode.commands.registerCommand(
    "hdlfmtr.formatDocument",
    () => {
      const editor = vscode.window.activeTextEditor;
      if (editor && editor.document.languageId === "vhdl") {
        vscode.commands.executeCommand("editor.action.formatDocument");
      }
    }
  );
  context.subscriptions.push(command);
}

export function deactivate() {
  diagnosticCollection?.dispose();
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

async function formatDocument(
  document: vscode.TextDocument
): Promise<vscode.TextEdit[]> {
  if (!wasmModule) {
    vscode.window.showErrorMessage(
      "hdlfmtr: WASM binary not loaded. Try reinstalling the extension."
    );
    return [];
  }

  const config = vscode.workspace.getConfiguration("hdlfmtr");
  const args = buildArgs(config, document.fileName);
  const source = document.getText();

  let result: WasiResult;
  try {
    result = await runWasm(wasmModule, args, source);
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    vscode.window.showErrorMessage(`hdlfmtr: ${msg}`);
    return [];
  }

  // Push lint diagnostics from stderr regardless of exit code.
  const diags = parseDiagnostics(result.stderr);
  diagnosticCollection.set(document.uri, diags);

  if (result.code !== 0) {
    vscode.window.showErrorMessage(
      `hdlfmtr exited with code ${result.code}: ${result.stderr.split("\n")[0] || "unknown error"}`
    );
    return [];
  }

  // Replace the entire document with the formatted output.
  const fullRange = new vscode.Range(
    document.positionAt(0),
    document.positionAt(source.length)
  );
  return [vscode.TextEdit.replace(fullRange, result.stdout)];
}

// ---------------------------------------------------------------------------
// CLI argument builder
// ---------------------------------------------------------------------------

function buildArgs(
  config: vscode.WorkspaceConfiguration,
  filepath: string
): string[] {
  const args = ["hdlfmtr", "--stdin-filepath", filepath];

  const indentStyle = config.get<string>("indentStyle", "tabs");
  args.push("--indent-style", indentStyle);

  const indentWidth = config.get<number>("indentWidth", 4);
  args.push("--indent-width", indentWidth.toString());

  const keywordCase = config.get<string>("keywordCase", "lower");
  args.push("--keyword-case", keywordCase);

  if (!config.get<boolean>("alignDeclarations", true)) {
    args.push("--no-align");
  }
  if (!config.get<boolean>("caseBranchIndent", true)) {
    args.push("--no-case-branch-indent");
  }
  if (!config.get<boolean>("sortUseClauses", true)) {
    args.push("--no-sort-uses");
  }
  if (!config.get<boolean>("lint", true)) {
    args.push("--no-lint");
  }

  return args;
}

// ---------------------------------------------------------------------------
// JSONL diagnostic parser
// ---------------------------------------------------------------------------

const SEVERITY_MAP: Record<string, vscode.DiagnosticSeverity> = {
  error: vscode.DiagnosticSeverity.Error,
  warning: vscode.DiagnosticSeverity.Warning,
  info: vscode.DiagnosticSeverity.Information,
  hint: vscode.DiagnosticSeverity.Hint,
};

function parseDiagnostics(stderr: string): vscode.Diagnostic[] {
  const diagnostics: vscode.Diagnostic[] = [];
  for (const line of stderr.split("\n")) {
    if (!line.trim()) {
      continue;
    }
    try {
      const d = JSON.parse(line);
      const range = new vscode.Range(
        new vscode.Position((d.line ?? 1) - 1, (d.col ?? 1) - 1),
        new vscode.Position(
          (d.end_line ?? d.line ?? 1) - 1,
          (d.end_col ?? d.col ?? 1) - 1
        )
      );
      const severity =
        SEVERITY_MAP[d.severity ?? "warning"] ??
        vscode.DiagnosticSeverity.Warning;
      const diag = new vscode.Diagnostic(range, d.message ?? "", severity);
      diag.source = "hdlfmtr";
      diag.code = d.code ?? undefined;
      diagnostics.push(diag);
    } catch {
      // Skip malformed lines.
    }
  }
  return diagnostics;
}
