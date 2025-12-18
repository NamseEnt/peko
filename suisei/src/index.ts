import type { AstroIntegration } from 'astro';
import { getAdapter } from './adapter.js';
import { createVitePlugins } from './vite/plugins.js';
import { buildServer } from './build.js';

export interface SuiseiOptions {}

export default function suisei(options: SuiseiOptions = {}): AstroIntegration {
  let _config: any;
  let _buildConfig: any;

  return {
    name: 'suisei',
    hooks: {
      'astro:config:setup': ({ updateConfig }) => {
        const vitePlugins = createVitePlugins();

        updateConfig({
          vite: {
            plugins: vitePlugins,
            ssr: {
              noExternal: ['hono', '@bytecodealliance/jco-std'],
            },
          },
        });
      },

      'astro:config:done': ({ config, setAdapter }) => {
        _config = config;
        _buildConfig = {
          client: config.build.client,
          server: config.build.server,
        };

        setAdapter(getAdapter());

        if (config.output === 'static') {
          console.warn(
            '⚠️  suisei adapter requires SSR mode. Set output: "server" in astro.config.mjs'
          );
        }
      },

      'astro:build:done': async () => {
        await buildServer(_config, _buildConfig);
      },
    },
  };
}
