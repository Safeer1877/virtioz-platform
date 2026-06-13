import logging
logging.basicConfig(level=logging.INFO)
try:
    from transformers import pipeline
    import torch
    print('loading...')
    p = pipeline('text-generation', model='TinyLlama/TinyLlama-1.1B-Chat-v1.0', torch_dtype=torch.float32, device=-1)
    print('done')
except Exception as e:
    print('ERROR:', e)
