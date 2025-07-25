## Access Control Simple
This example demonstrates how Authentication and Authorization works. The example contains Jupyter with Spark, Pyiceberg, trino and starrocks as query engines, OpenFGA as Authorization backend, Keycloak as IdP and Minio as storage. In this simple example, query engines are not shared between users. Please check the [`Access Control Advanced Example`](../access-control-advanced/) to see how authentication & authorization works even if trusted query engines are shared between users.

Run the example with the following command:
```bash
cd examples/access-control-simple
docker compose up
```

Now open your Browser:
* Jupyter: [http://localhost:8888](http://localhost:8888)
* Keycloak UI: [http://localhost:30080](http://localhost:30080)
* Swagger UI: [http://localhost:8181/swagger-ui/#/](http://localhost:8181/swagger-ui/#/) (Note that more endpoints are available than in the Minimal example as permissions are enabled)
* Lakekeeper UI (**Don't use this for bootstrapping**. Use the designated Notebook instead. Bootstrapping sets the initial admin user, which needs to be our technical user for the examples to work correctly.): [http://localhost:8181](http://localhost:8181)

Start by following the instructions in the `01-Bootstrap.ipynb` Notebook in Jupyter. After that, you can login to the [UI](http://localhost:8181) as:
* Username: `peter`
* Password: `iceberg`

A second user is also available which initially has no permissions:
* Username: `anna`
* Password: `iceberg`

You can also login to Keycloak using:
* Username: admin
* Password: admin

The Keycloak Realm "iceberg" is pre-configured.
