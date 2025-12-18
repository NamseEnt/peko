import { fileURLToPath } from 'node:url';
import { resolve, dirname } from 'node:path';
import { spawn, type SpawnOptions } from 'node:child_process';
import { promises as fs } from 'node:fs';
import { generateRolldownConfig } from './rolldown/config.js';

interface BuildConfig {
  server: URL;
  client: URL;
}

export async function buildServer(config: any, buildConfig: BuildConfig) {
  console.log('🚀 Building WASM component with suisei...');

  const serverDir = fileURLToPath(buildConfig.server);
  const clientDir = fileURLToPath(buildConfig.client);

  await createServerEntry(serverDir);

  const rolldownConfigPath = resolve(serverDir, 'rolldown.config.mjs');
  const componentInput = resolve(serverDir, 'component.ts');
  const componentOutput = resolve(serverDir, 'component.js');

  const rolldownConfig = generateRolldownConfig('./component.ts', './component.js');
  await fs.writeFile(rolldownConfigPath, rolldownConfig);

  const suiseiRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
  const witDir = resolve(suiseiRoot, 'wit');
  const rolldownBin = resolve(suiseiRoot, 'node_modules', '.bin', 'rolldown');
  const jcoBin = resolve(suiseiRoot, 'node_modules', '.bin', 'jco');

  console.log('📦 Running Rolldown bundler...');
  await runCommand(rolldownBin, ['-c', rolldownConfigPath], { cwd: serverDir });

  console.log('🔧 Running JCO componentization...');
  const wasmOutput = resolve(serverDir, 'component.wasm');
  const projectRoot = resolve(serverDir, '..', '..');

  const nodeModulesPath = resolve(projectRoot, 'node_modules');
  await runCommand(
    jcoBin,
    ['componentize', '-w', witDir, '-o', wasmOutput, componentOutput],
    {
      cwd: projectRoot,
      env: { ...process.env, NODE_PATH: nodeModulesPath }
    }
  );

  console.log('✅ WASM component built successfully!');
  console.log(`   Output: ${wasmOutput}`);
}

async function createServerEntry(serverDir: string) {
  const files = await fs.readdir(serverDir);
  const manifestFile = files.find((file) => file.startsWith('manifest_') && file.endsWith('.mjs'));

  if (!manifestFile) {
    throw new Error('Could not find manifest file in server directory');
  }

  const shimContent = `const IntlMock = {
  DateTimeFormat: class {
    constructor(locales, options) {}
    format(date) {
      return new Date(date || Date.now()).toISOString();
    }
    resolvedOptions() {
      return { locale: 'en-US' };
    }
  },
  NumberFormat: class {
    constructor(locales, options) {}
    format(number) {
      return String(number);
    }
    resolvedOptions() {
      return { locale: 'en-US' };
    }
  },
  Segmenter: class {
    segment(input) {
      return input.split('');
    }
  },
};

const WebAssemblyMock = {
  compile: () => Promise.reject(new Error('WebAssembly.compile is not supported')),
  instantiate: () => Promise.reject(new Error('WebAssembly.instantiate is not supported')),
  validate: () => false,
  Module: class {},
  Instance: class {},
  Memory: class {},
  Table: class {},
  CompileError: Error,
  LinkError: Error,
  RuntimeError: Error,
};

if (typeof globalThis.Intl === 'undefined') {
  globalThis.Intl = IntlMock;
}

if (typeof globalThis.WebAssembly === 'undefined') {
  globalThis.WebAssembly = WebAssemblyMock;
}
`;

  const entryContent = `import './shim-intl.js';
import { App } from 'astro/app';
import { manifest } from './${manifestFile}';
import { createHonoApp } from 'suisei/server/hono-app';
import { fire } from '@bytecodealliance/jco-std/wasi/0.2.6/http/adapters/hono/server';

const app = createHonoApp(new App(manifest));
fire(app);

export { incomingHandler } from '@bytecodealliance/jco-std/wasi/0.2.6/http/adapters/hono/server';
`;

  await fs.writeFile(resolve(serverDir, 'shim-intl.js'), shimContent);
  await fs.writeFile(resolve(serverDir, 'component.ts'), entryContent);
}

function runCommand(
  cmd: string,
  args: string[],
  options: SpawnOptions = {}
): Promise<void> {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, { stdio: 'inherit', ...options });
    child.on('close', (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(new Error(`Command failed with code ${code}`));
      }
    });
    child.on('error', reject);
  });
}
