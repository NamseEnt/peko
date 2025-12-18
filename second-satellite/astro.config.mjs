// @ts-check
import { defineConfig } from 'astro/config';
import suisei from 'suisei';

export default defineConfig({
  output: 'server',
  adapter: suisei(),
});
