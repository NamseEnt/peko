import type { Plugin } from 'vite';

export function createVitePlugins(): Plugin[] {
  return [esModuleLexerJSPlugin()];
}

function esModuleLexerJSPlugin(): Plugin {
  return {
    name: 'suisei:es-module-lexer-js',
    enforce: 'pre',

    config() {
      return {
        resolve: {
          alias: {
            'es-module-lexer': 'es-module-lexer/js',
          },
        },
      };
    },
  };
}
