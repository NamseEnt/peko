import type { AstroAdapter } from 'astro';

export function getAdapter(): AstroAdapter {
  return {
    name: 'suisei',
    serverEntrypoint: 'suisei/server-entry',
    exports: ['handler', 'incomingHandler'],
    supportedAstroFeatures: {
      hybridOutput: 'stable',
      staticOutput: 'stable',
      serverOutput: 'stable',
    },
  };
}
