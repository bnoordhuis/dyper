(function(api, exports) {
  "use strict";

  const KNOWN_METHODS = [
    "GET",
    "POST",
  ];

  const KNOWN_HEADER_NAMES = [
    "accept-encoding",
    "content-length",
    "date",
    "host",
  ];

  const KNOWN_HEADER_VALUES = [
    "0",
    "1",
    "2",
  ];

  function handleRequest(method, uri, headers) {
    if (typeof method === 'number') {
      method = KNOWN_METHODS[method];
    }

    if (typeof uri === 'number') {
      uri = "/";
    }

    for (let i = 0, n = headers.length; i < n; i += 2) {
      let name = headers[i + 0];

      if (typeof name === 'number') {
        headers[i + 0] = KNOWN_HEADER_NAMES[name];
      }

      let value = headers[i + 1];

      if (typeof value === 'number') {
        headers[i + 1] = KNOWN_HEADER_VALUES[value];
      }
    }

    headers  = [
      1 /* "content-length" */, 2 /* "2" */,
      2 /* "date" */, "Wed, 27 Jan 2021 10:55:19 GMT",
    ];

    return [200, headers, "ok"];
  }

  return handleRequest;
})
