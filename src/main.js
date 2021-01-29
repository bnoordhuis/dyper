(function(api, exports) {
  "use strict";

  function handleRequest(method, uri, headers) {
    return globalThis.onrequest(method, uri, headers);
  }

  return handleRequest;
})
