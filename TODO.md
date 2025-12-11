## list neighbors

- [ ] cloud api
  - [x] oci
  - [ ] aws
- [ ] db
- [ ] dns

## invoke user function

- [ ] over internet
- [ ] inter aws
  - [ ] aws with s3 payload
  - [ ] aws public ip in same region

## auto scaling, zombie killer

- [ ] cloudflare workers, d1, scheduling by master
- [ ] aws lambda, cron using aws eventbridge scheduler, state store using s3

## ping specific worker

- [ ] via public ip directly
- [ ] via cloudflare
- [ ] from aws using ipv6 egress only internet gateway

## rate limit, throttle

- [ ] worker level
- [ ] region level

## authentication, authorization

- [ ] create api key
- [ ] revoke api key
- [ ] whitelist store and cache

## queue support

- [ ] aws sqs
- [ ] redis
- [ ] Upstash QStash
- [ ] AMQP, CloudAMQP

## connection pooling

- [ ] http client
- [ ] tcp
  - [ ] pg
  - [ ] mysql
  - [ ] redis
  - [ ] application-agnostic

## cron

- [ ] pre-sharding + push by master.
- [ ] retrial
- [ ] refuse for scale-in

## document and homepage

- [ ] github pages + docusaurus + Algolia DocSearch
- [ ] one-click on-web deployment demo
- [ ] ls-news for showcase

## metrics monitoring

- [ ] grafana
- [ ] new relic
- [ ] oci monitoring

## server-side rendering support

- [ ] astro ssr

## deployment

- [ ] green-blue
- [ ] rolling

## operator dashboard

- [ ] manual operation
  - [ ] limit or set number of
    - [ ] scale in time
    - [ ] max terminated instance in time
    - [ ] scale out time
    - [ ] max instance count
    - [ ] max new instance in time
  - [ ] toggle zombie killing
  - [ ] toggle auto scaling

## client dashboard of fn0 cloud

## iaac

- [ ] pulumi

## code id

- [*] parse code id from url
- [ ] register asterisk domain in dns
- [ ] master(in-memory) code id whitelist to reduce storage request cost

## master worker election

- [ ] memberlist based, (birth time + id)'s oldest worker is master
- [ ] in-memory database

## outgoing http connection pool

- [ ] hard limit for each wasm instance

## health check

- [ ] endpoint
- [ ] status: graceful shutting down

## static file serving

- [ ] cloudflare r2 pulumi setting
- [ ] count GetObject by cloudflare graphql httpRequests1dGroups
- [ ] count CreateObject by deployment system
- [ ] deploy static files on cli
