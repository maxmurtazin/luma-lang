import * as vscode from 'vscode';
import {execFile} from 'node:child_process';
import {randomUUID} from 'node:crypto';
import * as fs from 'node:fs/promises';
import * as os from 'node:os';
import * as path from 'node:path';
import {promisify} from 'node:util';

const exec = promisify(execFile);
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const revision = /^[0-9]{1,20}$/;

class CadProblem extends Error {}

async function call(executable: string, args: string[]): Promise<Record<string, unknown>> {
    let stdout: string;
    try {
        ({stdout} = await exec(executable, args, {
            timeout: 90_000,
            maxBuffer: 8 * 1024 * 1024,
            windowsHide: true,
        }));
    } catch (failure) {
        const child = failure as {stdout?: unknown};
        if (typeof child.stdout !== 'string' || !child.stdout) {
            throw new CadProblem('The CAD process returned no receipt.');
        }
        stdout = child.stdout;
    }
    const result = JSON.parse(stdout);
    if (result.protocol !== 'luma.cad.core-project/1' || result.schema_version !== 1) {
        throw new CadProblem('The executable returned an unsupported CAD response.');
    }
    if (result.error) {
        const reason = result.error.details?.reason;
        const message = typeof reason === 'string' ? reason : '';
        throw new CadProblem([result.error.code, message].filter(Boolean).join(' · '));
    }
    if (!result.result || typeof result.result !== 'object') {
        throw new CadProblem('The CAD response has no result.');
    }
    return result.result;
}

async function show(value: unknown) {
    const document = await vscode.workspace.openTextDocument({
        language: 'json',
        content: JSON.stringify(value, null, 2),
    });
    await vscode.window.showTextDocument(document, {preview: true, preserveFocus: true});
}

export async function previewCadBundle() {
    let operation: string | undefined;
    try {
        if (!vscode.workspace.isTrusted || vscode.env.remoteName) {
            throw new CadProblem('CAD project commands require a trusted local workspace.');
        }
        const executable = vscode.workspace.getConfiguration('luma').get<string>('cadProjectExecutable');
        if (!executable || !path.isAbsolute(executable)) {
            throw new CadProblem('Set luma.cadProjectExecutable to the absolute path of luma-cad-project.');
        }
        if (!process.env.LUMA_CAD_PROJECT_KEY) {
            throw new CadProblem('Start VS Code with LUMA_CAD_PROJECT_KEY for this project.');
        }
        const mode = await vscode.window.showQuickPick(['recipe', 'command'], {placeHolder: 'CAD bundle mode'});
        if (!mode) return;
        const database = await vscode.window.showOpenDialog({title: 'Select the local CAD project database'});
        if (!database?.length || database[0].scheme !== 'file') return;
        const bundle = await vscode.window.showOpenDialog({
            title: 'Select a Luma source bundle',
            filters: {'Luma source bundle': ['json']},
        });
        if (!bundle?.length || bundle[0].scheme !== 'file') return;
        const project = await vscode.window.showInputBox({
            prompt: 'Project UUID',
            validateInput: value => uuid.test(value) ? undefined : 'Enter a lowercase project UUID.',
        });
        if (!project) return;
        const part = await vscode.window.showInputBox({
            prompt: 'Part UUID',
            validateInput: value => uuid.test(value) ? undefined : 'Enter a lowercase Part UUID.',
        });
        if (!part) return;
        const base = await vscode.window.showInputBox({
            prompt: 'Expected project revision',
            validateInput: value => revision.test(value) ? undefined : 'Enter a decimal project revision.',
        });
        if (!base) return;

        const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'luma-cad-preview-'));
        try {
            const token = path.join(temporary, 'candidate');
            const connection = [database[0].fsPath, project];
            const preview = await call(executable, [...connection, 'preview', mode, part, base, bundle[0].fsPath, token]);
            await show(preview);
            const confirm = await vscode.window.showInformationMessage(
                `Apply the prepared CAD change to Part ${part} at project revision ${base}?`,
                {modal: true}, 'Apply',
            );
            if (confirm !== 'Apply') return;
            const request = randomUUID().replaceAll('-', '');
            operation = request;
            let receipt;
            try {
                receipt = await call(executable, [...connection, 'apply', token, request]);
            } catch (failure) {
                // The checkpoint can have committed before the child lost its response.
                try {
                    receipt = await call(executable, [...connection, 'receipt', part, request]);
                } catch {
                    throw failure;
                }
            }
            await show({operation: request, ...receipt});
        } finally {
            await fs.rm(temporary, {recursive: true, force: true});
        }
    } catch (failure) {
        const problem = failure instanceof CadProblem ? failure.message : 'The CAD command could not complete.';
        await vscode.window.showErrorMessage(operation ? `${problem} Operation ${operation}` : problem);
    }
}
