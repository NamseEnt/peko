import { Hono } from 'hono';
import type { App } from 'astro/app';

export function createHonoApp(astroApp: App) {
  const app = new Hono();

  app.use('/*', async (c) => {
    const request = c.req.raw;
    const routeData = astroApp.match(request);

    if (routeData) {
      return await astroApp.render(request, { routeData });
    }

    return c.notFound();
  });

  return app;
}
