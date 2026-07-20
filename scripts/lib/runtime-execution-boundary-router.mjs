export function findRouterRequestStartCases(tokens) {
  return typescriptSwitchCases(
    tokens,
    [identifierToken('header'), punctuationToken('.'), identifierToken('type')],
    'request.start',
  );
}

export function findForwardKindTokenIndexes(tokens) {
  return tokenSequenceIndexes(tokens, [
    identifierToken('kind'),
    punctuationToken(':'),
    stringLiteralToken('forward'),
  ]);
}

const ROUTER_RELAY_OWNER_IDENTIFIERS = new Set([
  'handleRuntimeRequestStart',
  'RuntimeForwardInvocation',
  'forwardedRequestIdsByCaller',
  'trackForwardedRequest',
  'forwardResponseStart',
  'forwardResponseChunk',
  'forwardResponseEnd',
  'createForwardedRequestId',
  'resolveRuntimeOriginatedTimeoutMs',
  'validateRuntimeRequestStartSource',
]);

export function isRouterRelayOwnerToken(token) {
  return token.kind === 'identifier' && ROUTER_RELAY_OWNER_IDENTIFIERS.has(token.value);
}

export function inspectRouterServiceRejectionCase(tokens) {
  const missing = [];
  const guard = findServiceCallerGuard(tokens);
  if (!guard) {
    missing.push('service caller guard with throw');
  }
  const send = findTokenCall(tokens, [
    identifierToken('this'),
    punctuationToken('.'),
    identifierToken('sendFrame'),
  ]);
  if (!send) {
    missing.push('sendFrame call');
  }

  let responseType;
  let requestId;
  let errorObject;
  let errorCode;
  let errorMessage;
  if (send) {
    const payloadObject = findObjectArgument(tokens, send.open + 1, send.close);
    if (payloadObject) {
      responseType = findObjectProperty(
        tokens,
        payloadObject,
        'type',
        stringLiteralToken('response.error'),
      );
      requestId = findObjectProperty(tokens, payloadObject, 'requestId', [
        identifierToken('header'),
        punctuationToken('.'),
        identifierToken('requestId'),
      ]);
      errorObject = findObjectPropertyObject(tokens, payloadObject, 'error');
      if (errorObject) {
        errorCode = findObjectProperty(
          tokens,
          errorObject,
          'code',
          stringLiteralToken('InProcessServiceCallRequired'),
        );
        errorMessage = findObjectProperty(
          tokens,
          errorObject,
          'message',
          (token) => token.kind === 'literal'
            && token.literalKind === 'string'
            && token.value.includes('in-process binding'),
        );
      }
    }
  }
  if (!responseType) missing.push('response.error payload type');
  if (!requestId) missing.push('caller requestId projection');
  if (!errorObject) missing.push('error payload object');
  if (!errorCode) missing.push('stable error code');
  if (!errorMessage) missing.push('stable in-process error message');

  const returnIndex = tokenSequenceIndexes(
    tokens,
    [keywordToken('return'), punctuationToken(';')],
  ).find((index) => !send || index > send.close);
  if (returnIndex === undefined) {
    missing.push('return after send');
  }
  if (guard && send && guard.end >= send.start) {
    missing.push('service guard before send');
  }
  return { complete: missing.length === 0, missing };
}

export function isRouterRelayWorkToken(token) {
  if (!['identifier', 'keyword'].includes(token.kind)) {
    return false;
  }
  return /^(?:registry|runtimeRegistry|pickDispatchConnection|validateRuntimeRequestStartSource)$/i.test(token.value)
    || /(?:selection|pending|forward|lazy)/i.test(token.value);
}

function typescriptSwitchCases(tokens, switchExpression, label) {
  const matches = [];
  for (let index = 0; index < tokens.length; index += 1) {
    if (!tokenMatches(tokens[index], keywordToken('switch'))) {
      continue;
    }
    const openParen = index + 1;
    if (!tokenMatches(tokens[openParen], punctuationToken('('))) {
      continue;
    }
    const closeParen = matchingTokenDelimiterIndex(tokens, openParen, '(', ')');
    if (
      closeParen === -1
      || !tokenSliceMatches(tokens, switchExpression, openParen + 1, closeParen)
      || !tokenMatches(tokens[closeParen + 1], punctuationToken('{'))
    ) {
      continue;
    }
    const bodyOpen = closeParen + 1;
    const bodyClose = matchingTokenDelimiterIndex(tokens, bodyOpen, '{', '}');
    if (bodyClose === -1) {
      continue;
    }
    let braceDepth = 0;
    for (let cursor = bodyOpen + 1; cursor < bodyClose; cursor += 1) {
      const token = tokens[cursor];
      if (tokenMatches(token, punctuationToken('{'))) {
        braceDepth += 1;
        continue;
      }
      if (tokenMatches(token, punctuationToken('}'))) {
        braceDepth -= 1;
        continue;
      }
      if (
        braceDepth !== 0
        || !tokenMatches(token, keywordToken('case'))
        || !tokenMatches(tokens[cursor + 1], stringLiteralToken(label))
        || !tokenMatches(tokens[cursor + 2], punctuationToken(':'))
      ) {
        continue;
      }
      const end = sameLevelCaseEnd(tokens, cursor + 3, bodyClose);
      matches.push({ start: token.start, tokens: tokens.slice(cursor, end) });
      cursor = end - 1;
    }
    index = bodyClose;
  }
  return matches;
}

function sameLevelCaseEnd(tokens, start, bodyClose) {
  let braceDepth = 0;
  for (let index = start; index < bodyClose; index += 1) {
    const token = tokens[index];
    if (tokenMatches(token, punctuationToken('{'))) {
      braceDepth += 1;
    } else if (tokenMatches(token, punctuationToken('}'))) {
      braceDepth -= 1;
    } else if (
      braceDepth === 0
      && (tokenMatches(token, keywordToken('case')) || tokenMatches(token, keywordToken('default')))
    ) {
      return index;
    }
  }
  return bodyClose;
}

function findServiceCallerGuard(tokens) {
  for (let index = 0; index < tokens.length; index += 1) {
    if (
      !tokenMatches(tokens[index], keywordToken('if'))
      || !tokenMatches(tokens[index + 1], punctuationToken('('))
    ) {
      continue;
    }
    const close = matchingTokenDelimiterIndex(tokens, index + 1, '(', ')');
    if (close === -1 || !tokenSliceMatches(tokens, [
      identifierToken('header'),
      punctuationToken('.'),
      identifierToken('caller'),
      punctuationToken('.'),
      identifierToken('kind'),
      punctuationToken('!=='),
      stringLiteralToken('service'),
    ], index + 2, close)) {
      continue;
    }
    const bodyStart = close + 1;
    const bodyEnd = tokenMatches(tokens[bodyStart], punctuationToken('{'))
      ? matchingTokenDelimiterIndex(tokens, bodyStart, '{', '}')
      : statementEnd(tokens, bodyStart);
    if (
      bodyEnd !== -1
      && tokens.slice(bodyStart, bodyEnd + 1).some(
        (token) => tokenMatches(token, keywordToken('throw')),
      )
    ) {
      return { end: bodyEnd, start: index };
    }
  }
  return undefined;
}

function statementEnd(tokens, start) {
  for (let index = start; index < tokens.length; index += 1) {
    if (tokenMatches(tokens[index], punctuationToken(';'))) {
      return index;
    }
  }
  return -1;
}

function findTokenCall(tokens, callee) {
  for (const start of tokenSequenceIndexes(tokens, callee)) {
    const open = start + callee.length;
    if (!tokenMatches(tokens[open], punctuationToken('('))) {
      continue;
    }
    const close = matchingTokenDelimiterIndex(tokens, open, '(', ')');
    if (close !== -1) {
      return { close, open, start };
    }
  }
  return undefined;
}

function findObjectArgument(tokens, start, end) {
  for (let index = start; index < end; index += 1) {
    if (!tokenMatches(tokens[index], punctuationToken('{'))) {
      continue;
    }
    const close = matchingTokenDelimiterIndex(tokens, index, '{', '}');
    if (close !== -1 && close < end) {
      return { close, open: index };
    }
  }
  return undefined;
}

function findObjectProperty(tokens, object, name, expectedValue) {
  const pattern = [
    identifierToken(name),
    punctuationToken(':'),
    ...(Array.isArray(expectedValue) ? expectedValue : [expectedValue]),
  ];
  return tokenSequenceIndexes(tokens, pattern, object.open + 1, object.close)
    .find((index) => tokenIsAtObjectLevel(tokens, object.open, index));
}

function findObjectPropertyObject(tokens, object, name) {
  const start = tokenSequenceIndexes(
    tokens,
    [identifierToken(name), punctuationToken(':'), punctuationToken('{')],
    object.open + 1,
    object.close,
  ).find((index) => tokenIsAtObjectLevel(tokens, object.open, index));
  if (start === undefined) {
    return undefined;
  }
  const open = start + 2;
  const close = matchingTokenDelimiterIndex(tokens, open, '{', '}');
  return close === -1 || close >= object.close ? undefined : { close, open };
}

function tokenIsAtObjectLevel(tokens, objectOpen, index) {
  let braceDepth = 0;
  for (let cursor = objectOpen + 1; cursor < index; cursor += 1) {
    if (tokenMatches(tokens[cursor], punctuationToken('{'))) braceDepth += 1;
    if (tokenMatches(tokens[cursor], punctuationToken('}'))) braceDepth -= 1;
  }
  return braceDepth === 0;
}

function tokenSequenceIndexes(tokens, pattern, start = 0, end = tokens.length) {
  const indexes = [];
  for (let index = start; index + pattern.length <= end; index += 1) {
    if (pattern.every((expected, offset) => tokenMatches(tokens[index + offset], expected))) {
      indexes.push(index);
    }
  }
  return indexes;
}

function tokenSliceMatches(tokens, pattern, start, end) {
  return end - start === pattern.length
    && pattern.every((expected, offset) => tokenMatches(tokens[start + offset], expected));
}

function tokenMatches(token, expected) {
  return token !== undefined && expected(token);
}

function identifierToken(value) {
  return (token) => token.kind === 'identifier' && token.value === value;
}

function keywordToken(value) {
  return (token) => token.kind === 'keyword' && token.value === value;
}

function punctuationToken(value) {
  return (token) => token.kind === 'punctuation' && token.value === value;
}

function stringLiteralToken(value) {
  return (token) => token.kind === 'literal'
    && token.literalKind === 'string'
    && token.value === value;
}

function matchingTokenDelimiterIndex(tokens, openIndex, open, close) {
  let depth = 0;
  for (let index = openIndex; index < tokens.length; index += 1) {
    if (tokenMatches(tokens[index], punctuationToken(open))) {
      depth += 1;
    } else if (tokenMatches(tokens[index], punctuationToken(close))) {
      depth -= 1;
      if (depth === 0) {
        return index;
      }
    }
  }
  return -1;
}
