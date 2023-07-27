# Rate Limit API

This is a proof of concept quality project intended to demonstrate rate limiting API routes using an in memory store. In a production scale application this should be done with something like Redis with key expiration set to 60 seconds.
This example makes use of [Axum](https://docs.rs/axum/latest/axum/) 
and [evmap](https://docs.rs/evmap/latest/evmap/index.html)
to build an API that allows CRUD operations on an in memory KV store. The major challenge of using an in memory data structure as a store is supporting concurrent reads/writes potentially across multiple threads with limited latency.
In an attempt to achieve this goal I am passing a write handle wrapped in a Mutex [parking_lot](https://docs.rs/parking_lot/0.12.1/parking_lot/index.html)  and a read handle factory to all crud handlers. Both of these implementations will block while waiting to acquire the lock however, this introduces a minimal amount of latency when compared to operations on a standard HashMap when the number of keys exceed 10 million. 

## TTL
In order to facilitate a rudimentary ttl for each key in the EvMap a [priority_queue](https://docs.rs/priority-queue/latest/priority_queue/) is used when in the same background task that reconciles the EvMap. When an element with a ttl is added to the EvMap the ttl is also added to the queue.
This ensures that elements can be removed from the EvMap when they reach their ttl without needing to iterate the EvMap searching for expired items. 

## Usage

In an environment with cargo already installed the server can be started with

```bash
  cargo run
```

Or

```bash
  cargo run --release
```

The release build will be theoretically faster but given the minimal scale of the project the difference is fairly insignificant.
Then in another terminal the application can be tested with

```bash
curl -v -X POST localhost:3000/vault -H "Authorization: Bearer 1234"
curl -v -X PUT localhost:3000/vault/1 -H "Authorization: Bearer 1234"
curl -v localhost:3000/vault/items -H "Authorization: Bearer 1234"
```

Rate limits are set on a per route and api key basis. An api key (any valid string no validation is being done) may call one of the three routes up to the set limit for that route after which the route will return 429 and notify the caller how many seconds they must wait to call the route again. 

## Configuration

The server's port and rate limiting time may be adjusted from 3000 and 60 seconds respectively by adjusting the provided .env file. 
This will be picked up by the dotenv crate so calling `source .env` is unnecessary.
